//! 全屏 Space 的预览兼容层：WindowServer 有时无法取得 WKWebView 的远程图层。
//! 用 WebKit 的公开快照 API 缓存缩略图，只在全屏窗口失焦或被遮挡时显示。
//! 快照只留在内存，不截取其他窗口、不落盘，也不影响规则核心的后台计时。

use std::cell::{Cell, RefCell};
use std::ptr::{self, NonNull};
use std::rc::Rc;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{define_class, msg_send, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSImage, NSImageScaling, NSImageView, NSView, NSWindow,
    NSWindowDidBecomeKeyNotification, NSWindowDidChangeOcclusionStateNotification,
    NSWindowDidDeminiaturizeNotification, NSWindowDidEnterFullScreenNotification,
    NSWindowDidExitFullScreenNotification, NSWindowDidMiniaturizeNotification,
    NSWindowDidResignKeyNotification, NSWindowDidResizeNotification, NSWindowOcclusionState,
    NSWindowOrderingMode, NSWindowStyleMask,
};
use objc2_foundation::{
    NSError, NSNotification, NSNotificationCenter, NSNumber, NSObjectProtocol, NSOperationQueue,
    NSPoint, NSRect, NSTimer,
};
use objc2_web_kit::{WKSnapshotConfiguration, WKWebView};
use tauri::{AppHandle, Manager};

define_class!(
    #[unsafe(super(NSImageView))]
    #[thread_kind = MainThreadOnly]
    struct PreviewImageView;

    impl PreviewImageView {
        // 返回实时页面的第一下点击也必须透过快照，不能被装饰层吞掉。
        #[unsafe(method(hitTest:))]
        fn hit_test(&self, _point: NSPoint) -> *mut NSView { ptr::null_mut() }

        #[unsafe(method(isAccessibilityElement))]
        fn is_accessibility_element(&self) -> bool { false }
    }
);

#[derive(Default)]
struct SnapshotState {
    generation: u64,
    pending: Option<u64>,
    ready: bool,
}

impl SnapshotState {
    fn invalidate(&mut self) {
        self.generation += 1;
        self.ready = false;
        // 旧请求完成前不并发再截；完成时用 generation 拒绝旧尺寸的结果。
    }

    fn begin(&mut self) -> Option<u64> {
        if self.pending.is_some() {
            return None;
        }
        self.pending = Some(self.generation);
        self.pending
    }

    fn finish(&mut self, generation: u64, succeeded: bool) -> bool {
        if self.pending != Some(generation) {
            return false;
        }
        self.pending = None;
        let accepted = generation == self.generation && succeeded;
        self.ready |= accepted;
        accepted
    }
}

#[derive(Clone, Copy)]
struct WindowState {
    full_screen: bool,
    visible: bool,
    minimized: bool,
    key: bool,
    occluded: bool,
}

impl WindowState {
    fn eligible(self) -> bool {
        self.full_screen && self.visible && !self.minimized
    }
    fn show_preview(self, ready: bool) -> bool {
        self.eligible() && ready && (!self.key || self.occluded)
    }
    fn can_capture(self) -> bool {
        self.eligible() && self.key && !self.occluded
    }
}

struct Preview {
    webview: Retained<WKWebView>,
    window: Retained<NSWindow>,
    image: Retained<PreviewImageView>,
    bounds: Cell<NSRect>,
    snapshot: RefCell<SnapshotState>,
}

impl Preview {
    fn window_state(&self) -> WindowState {
        WindowState {
            full_screen: self
                .window
                .styleMask()
                .contains(NSWindowStyleMask::FullScreen),
            visible: self.window.isVisible(),
            minimized: self.window.isMiniaturized(),
            key: self.window.isKeyWindow(),
            occluded: !self
                .window
                .occlusionState()
                .contains(NSWindowOcclusionState::Visible),
        }
    }

    fn refresh(&self) {
        let bounds = self.webview.bounds();
        let state = self.window_state();
        if bounds != self.bounds.get() || !state.eligible() {
            self.bounds.set(bounds);
            self.snapshot.borrow_mut().invalidate();
            self.image.setImage(None);
        }
        self.image.setFrame(self.webview.frame());
        self.image
            .setHidden(!state.show_preview(self.snapshot.borrow().ready));
    }

    fn capture(self: &Rc<Self>) {
        self.refresh();
        if !self.window_state().can_capture() || unsafe { self.webview.isLoading() } {
            return;
        }
        let bounds = self.bounds.get();
        if !bounds.size.width.is_finite()
            || !bounds.size.height.is_finite()
            || bounds.size.width <= 0.0
            || bounds.size.height <= 0.0
        {
            return;
        }
        let Some(generation) = self.snapshot.borrow_mut().begin() else {
            return;
        };
        let weak = Rc::downgrade(self);
        let completed = RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
            let Some(preview) = weak.upgrade() else {
                return;
            };
            // WebKit 的完成回调在主线程；旧尺寸/退出全屏后的结果不能覆盖新页面。
            preview.refresh();
            let accepted = preview
                .snapshot
                .borrow_mut()
                .finish(generation, error.is_null() && !image.is_null());
            if accepted {
                preview.image.setImage(unsafe { image.as_ref() });
                preview.refresh();
            }
        });
        // 只需要桌面管理中的小图。限制长边，不为 4K 全屏保留大位图。
        let width = 480.0 * bounds.size.width / bounds.size.width.max(bounds.size.height);
        unsafe {
            let config = WKSnapshotConfiguration::new(self.webview.mtm());
            config.setRect(bounds);
            config.setSnapshotWidth(Some(&NSNumber::new_f64(width)));
            // 不等后台页面下一次绘制，避免切换 Space 时异步请求挂起。
            config.setAfterScreenUpdates(false);
            self.webview
                .takeSnapshotWithConfiguration_completionHandler(Some(&config), &completed);
        }
    }
}

struct Registration {
    preview: Rc<Preview>,
    timer: Retained<NSTimer>,
    observers: Vec<Retained<ProtocolObject<dyn NSObjectProtocol>>>,
}

impl Drop for Registration {
    fn drop(&mut self) {
        self.timer.invalidate();
        let center = NSNotificationCenter::defaultCenter();
        for observer in &self.observers {
            let token: &ProtocolObject<dyn NSObjectProtocol> = observer;
            unsafe {
                center.removeObserver(token.as_ref());
            }
        }
        self.preview.image.removeFromSuperview();
    }
}

thread_local! {
    // 所有 AppKit 对象及生命周期操作都留在主线程。
    static REGISTRATION: RefCell<Option<Registration>> = const { RefCell::new(None) };
}

pub fn install(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    if let Err(error) = window.with_webview(|platform| {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        // Tauri 明确保证 macOS 的 inner/ns_window 分别为 WKWebView / NSWindow。
        let (Some(webview), Some(window)) = (
            unsafe { Retained::retain(platform.inner().cast::<WKWebView>()) },
            unsafe { Retained::retain(platform.ns_window().cast::<NSWindow>()) },
        ) else {
            return;
        };
        let Some(parent) = (unsafe { webview.superview() }) else {
            return;
        };
        let allocated = PreviewImageView::alloc(mtm).set_ivars(());
        let image: Retained<PreviewImageView> =
            unsafe { msg_send![super(allocated), initWithFrame: webview.frame()] };
        image.setWantsLayer(true);
        image.setImageScaling(NSImageScaling::ScaleAxesIndependently);
        image.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        image.setHidden(true);
        parent.addSubview_positioned_relativeTo(
            &image,
            NSWindowOrderingMode::Above,
            Some(&webview),
        );
        let preview = Rc::new(Preview {
            bounds: Cell::new(webview.bounds()),
            webview,
            window,
            image,
            snapshot: RefCell::new(SnapshotState::default()),
        });

        let center = NSNotificationCenter::defaultCenter();
        let mut observers = Vec::new();
        // 显示状态变更立即处理；不能等五秒定时器才移除前台覆盖层。
        for name in unsafe {
            [
                NSWindowDidBecomeKeyNotification,
                NSWindowDidResignKeyNotification,
                NSWindowDidChangeOcclusionStateNotification,
                NSWindowDidResizeNotification,
                NSWindowDidEnterFullScreenNotification,
                NSWindowDidExitFullScreenNotification,
                NSWindowDidMiniaturizeNotification,
                NSWindowDidDeminiaturizeNotification,
            ]
        } {
            let weak = Rc::downgrade(&preview);
            let changed = RcBlock::new(move |_notification: NonNull<NSNotification>| {
                if let Some(preview) = weak.upgrade() {
                    preview.refresh();
                    if !preview.snapshot.borrow().ready {
                        preview.capture();
                    }
                }
            });
            // 监听指定窗口，回调明确排到主队列，Rc 与 AppKit 句柄不跨线程。
            observers.push(unsafe {
                center.addObserverForName_object_queue_usingBlock(
                    Some(name),
                    Some(&preview.window),
                    Some(&NSOperationQueue::mainQueue()),
                    &changed,
                )
            });
        }
        let weak = Rc::downgrade(&preview);
        let tick = RcBlock::new(move |_timer: NonNull<NSTimer>| {
            if let Some(preview) = weak.upgrade() {
                preview.capture();
            }
        });
        // 只在前台全屏时每五秒更新一次；后台不持续截图、不关闭 WebKit 的省电机制。
        let timer =
            unsafe { NSTimer::scheduledTimerWithTimeInterval_repeats_block(5.0, true, &tick) };
        timer.setTolerance(1.0);
        preview.capture();
        REGISTRATION.with(|slot| {
            *slot.borrow_mut() = Some(Registration {
                preview,
                timer,
                observers,
            })
        });
    }) {
        eprintln!("sitzfleisch: 全屏预览未启用：{error}");
    }
}

pub fn uninstall() {
    REGISTRATION.with(|slot| {
        slot.borrow_mut().take();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn foreground() -> WindowState {
        WindowState {
            full_screen: true,
            visible: true,
            minimized: false,
            key: true,
            occluded: false,
        }
    }

    #[test]
    fn foreground_never_shows_snapshot_and_background_never_captures() {
        let active = foreground();
        assert!(active.can_capture());
        assert!(!active.show_preview(true));
        for background in [
            WindowState {
                key: false,
                ..active
            },
            WindowState {
                occluded: true,
                ..active
            },
        ] {
            assert!(background.show_preview(true));
            assert!(!background.show_preview(false));
            assert!(!background.can_capture());
        }
    }

    #[test]
    fn normal_hidden_and_minimized_windows_never_show_or_capture() {
        let active = foreground();
        for state in [
            WindowState {
                full_screen: false,
                ..active
            },
            WindowState {
                visible: false,
                ..active
            },
            WindowState {
                minimized: true,
                ..active
            },
        ] {
            assert!(!state.show_preview(true));
            assert!(!state.can_capture());
        }
    }

    #[test]
    fn resize_rejects_late_snapshot_without_starting_parallel_requests() {
        let mut state = SnapshotState::default();
        let first = state.begin().unwrap();
        state.invalidate();
        assert!(state.begin().is_none());
        assert!(!state.finish(first, true));
        assert!(!state.ready);
        let resized = state.begin().unwrap();
        assert!(state.finish(resized, true));
        assert!(state.ready);
    }

    #[test]
    fn failed_refresh_keeps_last_good_image_and_allows_retry() {
        let mut state = SnapshotState::default();
        let first = state.begin().unwrap();
        assert!(!state.finish(first, false));
        assert!(!state.ready);
        let second = state.begin().unwrap();
        assert!(state.finish(second, true));
        let third = state.begin().unwrap();
        assert!(!state.finish(third, false));
        assert!(state.ready);
        assert!(state.begin().is_some());
    }
}

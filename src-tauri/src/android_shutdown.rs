#![cfg_attr(not(target_os = "android"), allow(dead_code))]

use std::sync::{atomic::{AtomicBool, Ordering}, Arc, Mutex};
use std::thread::{self, JoinHandle};

/// 调用前必须已停止心跳并完成最终保存。Android 虚拟机的其他线程仍在运行，
/// libc exit 的静态析构会与它们争用已销毁的 C++ 互斥锁；_exit 保留退出码而跳过析构。
#[cfg(target_os = "android")]
pub fn finish_android_process() -> ! {
    unsafe extern "C" { fn _exit(status: std::ffi::c_int) -> !; }
    // NDK unistd.h 中 _exit(int) 为 noreturn；这里没有外部参数，也不再持有文件或状态锁。
    unsafe { _exit(0) }
}

/// Android 销毁 Activity 会退出事件循环；先结束心跳，避免退出后仍唤醒已释放的原生循环。
#[derive(Default)]
pub struct HeartbeatShutdown {
    requested: AtomicBool,
    stopped: AtomicBool,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl HeartbeatShutdown {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_worker(&self, worker: JoinHandle<()>) {
        *self.worker.lock().unwrap() = Some(worker);
    }

    pub fn stop_requested(&self) -> bool {
        self.requested.load(Ordering::SeqCst)
    }

    /// 返回 true 时暂缓退出。等待放在另一个线程，让事件循环继续处理尚未返回的原生调用。
    pub fn request_exit(self: &Arc<Self>, after_stopped: impl FnOnce() + Send + 'static) -> bool {
        if self.stopped.load(Ordering::SeqCst) { return false; }
        if !self.requested.swap(true, Ordering::SeqCst) {
            let worker = self.worker.lock().unwrap().take();
            if let Some(worker) = &worker { worker.thread().unpark(); }
            let shutdown = Arc::clone(self);
            thread::spawn(move || {
                if let Some(worker) = worker { let _ = worker.join(); }
                shutdown.stopped.store(true, Ordering::SeqCst);
                after_stopped();
            });
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::{channel, TryRecvError};
    use std::time::Duration;

    #[test]
    fn exit_waits_for_in_flight_native_work_without_blocking_the_event_loop() {
        let shutdown = Arc::new(HeartbeatShutdown::new());
        let (ready_tx, ready_rx) = channel();
        let (reply_tx, reply_rx) = channel();
        let (finished_tx, finished_rx) = channel();
        shutdown.register_worker(thread::spawn(move || {
            ready_tx.send(()).unwrap();
            reply_rx.recv().unwrap();
        }));
        ready_rx.recv().unwrap();
        assert!(shutdown.request_exit(move || { finished_tx.send(()).unwrap(); }));
        assert!(shutdown.stop_requested());
        assert!(shutdown.request_exit(|| panic!("重复退出不能另开清理线程")));
        assert_eq!(finished_rx.try_recv(), Err(TryRecvError::Empty));
        // 模拟事件循环处理完最后一个 JNI 回调；调用 request_exit 的线程一直能继续推进。
        reply_tx.send(()).unwrap();
        finished_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(!shutdown.request_exit(|| panic!("已结束后应直接允许退出")));
    }

    #[test]
    fn exit_unparks_an_idle_heartbeat_and_waits_for_its_last_work() {
        let shutdown = Arc::new(HeartbeatShutdown::new());
        let worker_shutdown = Arc::clone(&shutdown);
        let (ready_tx, ready_rx) = channel();
        let (finished_tx, finished_rx) = channel();
        shutdown.register_worker(thread::spawn(move || {
            ready_tx.send(()).unwrap();
            while !worker_shutdown.stop_requested() { thread::park(); }
        }));
        ready_rx.recv().unwrap();
        assert!(!shutdown.stop_requested());
        assert!(shutdown.request_exit(move || { finished_tx.send(()).unwrap(); }));
        finished_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(!shutdown.request_exit(|| {}));
    }
}

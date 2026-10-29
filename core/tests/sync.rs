use sitzfleisch_core::{
    sync::{
        merge, serialized_jsonb_len, MergeChoice, SyncDocument, MAX_DOCUMENT_BYTES,
        MAX_HISTORY_DAYS,
    },
    State,
};

const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const C: &str = "cccccccccccccccccccccccccccccccc";

fn capture(state: &mut State, previous: Option<&SyncDocument>, namespace: &str) -> SyncDocument {
    SyncDocument::capture(state, previous, namespace, &[]).unwrap()
}

fn running() -> (State, SyncDocument) {
    let mut state = State::new(1_000);
    state.start_day("standard", 1_000).unwrap();
    state
        .start_block_with_break("main", 1, Vec::new(), 0)
        .unwrap();
    let document = capture(&mut state, None, A);
    (state, document)
}

fn archived(at: i64, namespace: &str) -> (State, SyncDocument) {
    let mut state = State::new(at);
    state.start_day("standard", at).unwrap();
    state
        .start_block_with_break("main", 1, Vec::new(), 0)
        .unwrap();
    let initial = capture(&mut state, None, namespace);
    state.tick_wall_clock(at + 60);
    state.end_day(at + 60).unwrap();
    let document = capture(&mut state, Some(&initial), namespace);
    (state, document)
}

fn json<T: serde::Serialize>(value: &T) -> serde_json::Value {
    serde_json::to_value(value).unwrap()
}

#[test]
fn standalone_serialization_keeps_sync_ids_absent_and_old_schema() {
    let mut state = State::new(1_000);
    state.start_day("standard", 1_000).unwrap();
    state.start_block("main", 1, Vec::new()).unwrap();
    let before = sitzfleisch_core::to_json(&state);
    assert!(!before.contains("sync_id"));
    assert_eq!(state.schema, 5);
    assert_eq!(
        json(&state),
        json(&sitzfleisch_core::from_json(&before).unwrap())
    );
}

#[test]
fn migration_allocates_and_persists_ids_and_keeps_them_across_capture() {
    let (mut state, original) = running();
    let persisted = sitzfleisch_core::to_json(&state);
    let mut restarted = sitzfleisch_core::from_json(&persisted).unwrap();
    let after = capture(&mut restarted, Some(&original), B);
    assert_eq!(json(&original), json(&after));
    assert_eq!(
        state.day.as_ref().unwrap().sync_id,
        after.day.as_ref().unwrap().sync_id
    );
    state.tick_wall_clock(1_060);
    let completed = capture(&mut state, Some(&original), C);
    assert_eq!(
        original
            .day
            .as_ref()
            .unwrap()
            .timer
            .as_ref()
            .unwrap()
            .sync_id,
        completed.day.as_ref().unwrap().ledger[0].sync_id
    );
}

#[test]
fn shared_document_contains_no_device_preferences_or_reminder_counters() {
    let (mut state, _) = running();
    state.preferences.blocked_hosts = vec!["example.com".into()];
    state.preferences.sound_enabled = false;
    state.preferences.water_reminder_enabled = true;
    state.preferences.stretch_reminder_enabled = true;
    state.preferences.app_blocking.enabled = true;
    state.tick_wall_clock(1_030);
    let document = capture(&mut state, None, A);
    let raw = serde_json::to_string(&document).unwrap();
    for field in [
        "blocked_hosts",
        "blocked_urls",
        "app_blocking",
        "sound_enabled",
        "water_reminder_enabled",
        "water_clock_checked_at",
        "water_reminded_at",
    ] {
        assert!(!raw.contains(field), "{field}");
    }
    assert_eq!(document.day.as_ref().unwrap().seated_since_relief, 0);
    assert_eq!(
        state.day.as_ref().unwrap().seated_since_relief,
        30,
        "网络投影不能清掉设备自身的提醒计数"
    );
}

#[test]
fn apply_preserves_device_preferences_and_water_deduplication() {
    let (_, document) = running();
    let mut phone = State::new(900);
    phone.preferences.water_reminder_enabled = true;
    phone.preferences.sound_enabled = false;
    phone.preferences.blocked_hosts = vec!["example.com".into()];
    phone.preferences.app_blocking.enabled = true;
    phone.water_clock_checked_at = Some(880);
    phone.water_reminded_at = Some(850);
    document.apply(&mut phone, 1_020).unwrap();
    assert!(phone.preferences.water_reminder_enabled);
    assert!(!phone.preferences.sound_enabled);
    assert_eq!(phone.preferences.blocked_hosts, ["example.com"]);
    assert!(phone.preferences.app_blocking.enabled);
    assert_eq!(phone.water_clock_checked_at, Some(880));
    assert_eq!(phone.water_reminded_at, Some(850));
    assert_eq!(
        phone
            .day
            .as_ref()
            .unwrap()
            .timer
            .as_ref()
            .unwrap()
            .elapsed_seconds,
        20
    );
    phone.day.as_mut().unwrap().seated_since_relief = 17;
    document.apply(&mut phone, 1_025).unwrap();
    assert_eq!(phone.day.as_ref().unwrap().seated_since_relief, 17);
}

#[test]
fn mac_and_phone_project_the_same_finished_record_only_once() {
    let (_, document) = running();
    let mut mac = State::new(0);
    let mut phone = State::new(0);
    document.apply(&mut mac, 1_200).unwrap();
    document.apply(&mut phone, 1_200).unwrap();
    assert_eq!(json(&mac.day), json(&phone.day));
    let day = mac.day.as_ref().unwrap();
    assert_eq!(day.ledger.len(), 1);
    assert_eq!(day.ledger[0].seconds, 60);
    assert_eq!(day.ledger[0].ended_at, 1_060);
    assert_eq!(day.suspend_seconds, 0);
    document.apply(&mut mac, 1_300).unwrap();
    assert_eq!(mac.day.as_ref().unwrap().ledger.len(), 1);
}

#[test]
fn natural_ticks_reminder_consumption_and_expired_breaks_are_equivalent() {
    let (mut state, base) = running();
    state.preferences.stretch_reminder_enabled = true;
    state.preferences.stretch_reminder_minutes = 5;
    state.tick_wall_clock(1_130);
    state.take_due_reminders();
    state.take_due_break(1_130);
    let local = capture(&mut state, Some(&base), B);
    assert!(base.equivalent(&local, 1_130));
    assert!(merge(&base, &local, &base, 1_130, None)
        .unwrap()
        .equivalent(&base, 1_130));
}

#[test]
fn clock_rollback_does_not_rewind_the_shared_anchor() {
    let (_, base) = running();
    let future = base.projected(1_020).unwrap();
    let projected = future.projected(999).unwrap();
    assert_eq!(projected.last_tick, 1_020);
    assert_eq!(
        projected
            .day
            .as_ref()
            .unwrap()
            .timer
            .as_ref()
            .unwrap()
            .elapsed_seconds,
        20
    );
}

#[test]
fn a_pause_is_applied_on_both_devices_without_advancing_elapsed_time() {
    let (mut state, base) = running();
    state.tick_wall_clock(1_020);
    state.toggle_pause(1_020).unwrap();
    let pause = capture(&mut state, Some(&base), B);
    let combined = merge(&base, &pause, &base, 1_050, None).unwrap();
    let day = combined.day.unwrap();
    assert!(day.is_paused());
    assert_eq!(day.timer.unwrap().elapsed_seconds, 20);
}

#[test]
fn independent_history_records_merge_and_both_orders_converge() {
    let mut empty = State::new(1_000);
    let base = capture(&mut empty, None, A);
    let (_, local) = archived(1_000, A);
    let (_, remote) = archived(2_000, B);
    let joined = merge(&base, &local, &remote, 3_000, None).unwrap();
    let reverse = merge(&base, &remote, &local, 3_000, None).unwrap();
    assert_eq!(joined.history.len(), 2);
    assert_eq!(json(&joined), json(&reverse));
}

#[test]
fn independent_preference_fields_merge_without_overwriting_the_other_edit() {
    let mut state = State::new(1_000);
    let base = capture(&mut state, None, A);
    let mut local = base.clone();
    local.preferences.break_minutes = 5;
    let mut remote = base.clone();
    remote.preferences.uniform_block_minutes = 40;
    let joined = merge(&base, &local, &remote, 1_000, None).unwrap();
    assert_eq!(joined.preferences.break_minutes, 5);
    assert_eq!(joined.preferences.uniform_block_minutes, 40);
}

#[test]
fn conflicting_current_timer_requires_choice_and_retains_independent_history() {
    let (mut local_state, base) = running();
    let mut remote_state = local_state.clone();
    local_state.tick_wall_clock(1_010);
    local_state.toggle_pause(1_010).unwrap();
    let local = capture(&mut local_state, Some(&base), B);
    remote_state.tick_wall_clock(1_020);
    remote_state.extend_block(1).unwrap();
    let (_, other_day) = archived(500, C);
    remote_state.history = other_day.history;
    let remote = capture(&mut remote_state, Some(&base), C);
    assert_eq!(
        merge(&base, &local, &remote, 1_030, None)
            .unwrap_err()
            .fields,
        ["day"]
    );
    let chosen = merge(&base, &local, &remote, 1_030, Some(MergeChoice::Local)).unwrap();
    assert!(chosen.day.as_ref().unwrap().is_paused());
    assert_eq!(
        chosen.history.len(),
        1,
        "选择当前计时不能丢掉另一端独立的历史"
    );
}

#[test]
fn ending_a_day_is_not_a_conflict_with_the_other_device_merely_ticking() {
    let (mut state, base) = running();
    state.tick_wall_clock(1_030);
    state.end_day(1_030).unwrap();
    let local = capture(&mut state, Some(&base), B);
    let merged = merge(&base, &local, &base, 1_040, None).unwrap();
    assert!(merged.day.is_none());
    assert_eq!(merged.history.len(), 1);
    assert_eq!(merged.history[0].day.ledger[0].seconds, 30);
}

#[test]
fn selecting_current_day_after_concurrent_archive_does_not_duplicate_it() {
    let (mut local_state, base) = running();
    let mut remote_state = local_state.clone();
    local_state.tick_wall_clock(1_030);
    local_state.end_day(1_030).unwrap();
    let local = capture(&mut local_state, Some(&base), B);
    remote_state.tick_wall_clock(1_020);
    remote_state.toggle_pause(1_020).unwrap();
    let remote = capture(&mut remote_state, Some(&base), C);
    assert!(merge(&base, &local, &remote, 1_040, None).is_err());
    let selected = merge(&base, &local, &remote, 1_040, Some(MergeChoice::Remote)).unwrap();
    assert!(selected.day.is_some());
    assert!(selected.history.is_empty());
}

#[test]
fn deletion_marker_prevents_unchanged_old_history_from_reappearing() {
    let (mut state, base) = archived(1_000, A);
    let id = state.history[0].day.sync_id.clone();
    state.delete_history_day(1_000).unwrap();
    let local =
        SyncDocument::capture(&mut state, Some(&base), B, std::slice::from_ref(&id)).unwrap();
    let joined = merge(&base, &local, &base, 1_200, None).unwrap();
    assert!(joined.history.is_empty());
    assert_eq!(joined.deleted_day_ids, [id]);
}

#[test]
fn concurrent_history_edit_and_delete_is_explicitly_resolved() {
    let (mut state, base) = archived(1_000, A);
    let mut remote = base.clone();
    remote.history[0].day.ledger[0].completion_note = Some("另一端补写的内容".into());
    let id = state.history[0].day.sync_id.clone();
    state.delete_history_day(1_000).unwrap();
    let local = SyncDocument::capture(&mut state, Some(&base), B, &[id]).unwrap();
    assert!(merge(&base, &local, &remote, 1_200, None).is_err());
    let deleted = merge(&base, &local, &remote, 1_200, Some(MergeChoice::Local)).unwrap();
    assert!(deleted.history.is_empty());
    let retained = merge(&base, &local, &remote, 1_200, Some(MergeChoice::Remote)).unwrap();
    assert_eq!(retained.history.len(), 1);
    assert!(retained.deleted_day_ids.is_empty());
}

#[test]
fn automatic_local_history_trimming_is_not_cloud_deletion() {
    let mut state = State::new(1_000);
    let mut previous = capture(&mut state, None, A);
    for index in 0..65 {
        let now = 1_000 + index * 120;
        state.start_day("standard", now).unwrap();
        state
            .start_block_with_break("main", 1, Vec::new(), 0)
            .unwrap();
        previous = capture(&mut state, Some(&previous), A);
        state.tick_wall_clock(now + 60);
        state.end_day(now + 60).unwrap();
        previous = capture(&mut state, Some(&previous), A);
    }
    assert_eq!(state.history.len(), 60);
    assert_eq!(previous.history.len(), 65);
    assert!(previous.deleted_day_ids.is_empty());
    previous.validate().unwrap();
}

#[test]
fn importing_the_same_legacy_archive_reuses_known_ids() {
    let (mut legacy, remote) = archived(1_000, A);
    legacy.history[0].day.sync_id.clear();
    for entry in &mut legacy.history[0].day.ledger {
        entry.sync_id.clear();
    }
    let imported = capture(&mut legacy, Some(&remote), B);
    assert_eq!(imported.history.len(), 1);
    assert_eq!(json(&imported.history), json(&remote.history));
    assert_eq!(legacy.history[0].day.sync_id, remote.history[0].day.sync_id);
}

#[test]
fn invalid_capture_and_apply_are_atomic() {
    let (mut state, base) = running();
    let before = json(&state);
    assert!(SyncDocument::capture(&mut state, None, "../invalid", &[]).is_err());
    assert_eq!(json(&state), before);
    let mut invalid = base.clone();
    invalid
        .day
        .as_mut()
        .unwrap()
        .timer
        .as_mut()
        .unwrap()
        .total_seconds = i64::MAX;
    assert!(invalid.apply(&mut state, 1_001).is_err());
    assert_eq!(json(&state), before);
    assert!(!invalid.equivalent(&base, 1_001));
}

#[test]
fn strict_network_parser_rejects_nested_unknown_fields_and_missing_keys() {
    let (_, base) = running();
    assert_eq!(
        json(&SyncDocument::from_json(&serde_json::to_string(&base).unwrap()).unwrap()),
        json(&base)
    );
    for pointer in [
        "",
        "/preferences",
        "/preferences/categories/0",
        "/day",
        "/day/timer",
        "/day/categories/0",
    ] {
        let mut extra = json(&base);
        extra
            .pointer_mut(pointer)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("access_token".into(), "must-not-travel".into());
        assert!(
            SyncDocument::from_json(&extra.to_string()).is_err(),
            "{pointer}"
        );
    }
    let mut missing = json(&base);
    missing["day"].as_object_mut().unwrap().remove("rests");
    assert!(SyncDocument::from_json(&missing.to_string()).is_err());
}

#[test]
fn strict_network_parser_rejects_invalid_ids_references_totals_and_bounds() {
    let (_, base) = running();
    let invalids = [
        ("/schema", json(&2)),
        ("/last_tick", json(&i64::MAX)),
        ("/day/sync_id", json(&"../../escape")),
        ("/day/timer/category", json(&"missing")),
        ("/day/timer/elapsed_seconds", json(&61)),
        ("/day/timer/total_seconds", json(&0)),
        ("/day/categories/0/accepted_seconds", json(&1)),
        ("/day/paused_seconds", json(&-1)),
        ("/day/seated_since_relief", json(&10)),
        ("/preferences/categories/0/name", json(&"x".repeat(81))),
        ("/preferences/profiles/0/id", json(&"")),
        ("/day/profile_name", json(&"\u{0}name")),
    ];
    for (pointer, value) in invalids {
        let mut bad = json(&base);
        *bad.pointer_mut(pointer).unwrap() = value;
        assert!(
            SyncDocument::from_json(&bad.to_string()).is_err(),
            "{pointer}"
        );
    }
    let mut duplicate = base.clone();
    duplicate
        .day
        .as_mut()
        .unwrap()
        .timer
        .as_mut()
        .unwrap()
        .sync_id = duplicate.day.as_ref().unwrap().sync_id.clone();
    assert!(duplicate.validate().is_err());
}

#[test]
fn capacities_reject_instead_of_silently_truncating_or_allocating_unbounded_data() {
    let (_, mut history) = archived(1_000, A);
    history.history = vec![history.history[0].clone(); MAX_HISTORY_DAYS + 1];
    assert!(history.validate().is_err());
    assert!(SyncDocument::from_json(&" ".repeat(MAX_DOCUMENT_BYTES + 1)).is_err());
    let (_, mut document) = running();
    document.deleted_day_ids = vec![format!("{B}-d-0"); 10_001];
    assert!(document.validate().is_err());
}

#[test]
fn jsonb_byte_count_includes_only_structural_spaces_and_utf8_bytes() {
    // PostgreSQL JSONB 的实际文本形式。字符串里的标点不增加结构空格；中文按 UTF-8 字节算。
    let postgres_text = r#"{"array": [1, 2, {"x": "a,b:c"}], "empty": {}, "text": "汉字\"\\\n"}"#;
    let value: serde_json::Value = serde_json::from_str(postgres_text).unwrap();
    assert_eq!(serialized_jsonb_len(&value).unwrap(), postgres_text.len());
    assert_eq!(serialized_jsonb_len(&serde_json::json!([])).unwrap(), 2);
    assert_eq!(
        serialized_jsonb_len(&serde_json::json!([true, false, null])).unwrap(),
        19
    );
    assert!(serialized_jsonb_len(&serde_json::json!({"text": "\0"})).is_err());
    assert!(serialized_jsonb_len(&serde_json::json!({"\0": "text"})).is_err());
}

#[test]
fn postgres_jsonb_limit_is_enforced_even_when_compact_json_is_smaller() {
    let (_, mut document) = archived(1_000, A);
    let template = document.history[0].clone();
    document.history = (0..800)
        .map(|index| {
            let mut archived = template.clone();
            archived.day.sync_id = format!("{A}-d-{index}");
            archived.day.ledger[0].sync_id = format!("{A}-e-{index}");
            archived.day.ledger[0].completion_note = Some(String::new());
            archived
        })
        .collect();
    let mut remaining = MAX_DOCUMENT_BYTES - serialized_jsonb_len(&document).unwrap();
    for archived in &mut document.history {
        let padding = remaining.min(2_000);
        archived.day.ledger[0].completion_note = Some("x".repeat(padding));
        remaining -= padding;
    }
    assert_eq!(remaining, 0, "边界夹具应有足够合法文本容量");
    assert_eq!(serialized_jsonb_len(&document).unwrap(), MAX_DOCUMENT_BYTES);
    document.validate().unwrap();
    let at_limit = serde_json::to_string(&document).unwrap();
    assert!(at_limit.len() < MAX_DOCUMENT_BYTES);
    SyncDocument::from_json(&at_limit).unwrap();

    document
        .history
        .iter_mut()
        .find_map(|archived| {
            let note = archived.day.ledger[0].completion_note.as_mut().unwrap();
            if note.len() < 2_000 {
                note.push('x');
                Some(())
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(
        serialized_jsonb_len(&document).unwrap(),
        MAX_DOCUMENT_BYTES + 1
    );
    let over_limit = serde_json::to_string(&document).unwrap();
    assert!(
        over_limit.len() < MAX_DOCUMENT_BYTES,
        "紧凑 JSON 的原始长度不足以代表服务端的上限"
    );
    assert!(document.validate().is_err());
    assert!(SyncDocument::from_json(&over_limit).is_err());
}

#[test]
fn nul_is_rejected_but_supported_multiline_notes_survive_network_roundtrip() {
    let (_, mut document) = archived(1_000, A);
    document.history[0].day.ledger[0].completion_note = Some("第一行\n第二行\t缩进\r旧换行".into());
    let serialized = serde_json::to_string(&document).unwrap();
    assert_eq!(
        json(&SyncDocument::from_json(&serialized).unwrap()),
        json(&document)
    );
    document.history[0].day.ledger[0].completion_note = Some("before\0after".into());
    assert!(document.validate().is_err());
    assert!(SyncDocument::from_json(&serde_json::to_string(&document).unwrap()).is_err());
}

#[test]
fn backend_contract_fixture_is_accepted_by_the_core_protocol() {
    let fixture = include_str!("../../supabase/tests/fixtures/document.json");
    let document = SyncDocument::from_json(fixture).unwrap();
    assert_eq!(
        json(&document),
        serde_json::from_str::<serde_json::Value>(fixture).unwrap()
    );
    assert_eq!(serialized_jsonb_len(&document).unwrap(), 1459);
}

#[test]
fn stable_history_targeting_preserves_other_device_records_started_in_the_same_second() {
    let (mut state, _) = archived(1_000, A);
    let (other, _) = archived(1_000, B);
    let original_id = state.history[0].day.sync_id.clone();
    let other_id = other.history[0].day.sync_id.clone();
    let other_entry_id = other.history[0].day.ledger[0].sync_id.clone();
    state.history.extend(other.history);

    state
        .set_completion_note_by_id(&other_id, &other_entry_id, " 手机完成\r\n第二行 ")
        .unwrap();
    assert_eq!(state.history[0].day.ledger[0].completion_note, None);
    assert_eq!(
        state.history[1].day.ledger[0].completion_note.as_deref(),
        Some("手机完成\n第二行")
    );
    state.delete_history_day_by_id(&other_id).unwrap();
    assert_eq!(state.history.len(), 1);
    assert_eq!(state.history[0].day.sync_id, original_id);
}

#[test]
fn stable_completion_target_survives_entry_reordering_and_cannot_fall_back_to_time() {
    let (mut state, initial) = running();
    state.tick_wall_clock(1_030);
    state.finish_block(1_030).unwrap();
    state
        .start_block_with_break("main", 1, Vec::new(), 0)
        .unwrap();
    let before_finish = capture(&mut state, Some(&initial), B);
    state.tick_wall_clock(1_060);
    state.finish_block(1_060).unwrap();
    capture(&mut state, Some(&before_finish), C);
    let day = state.day.as_mut().unwrap();
    let day_id = day.sync_id.clone();
    let first_id = day.ledger[0].sync_id.clone();
    day.ledger.swap(0, 1);
    state
        .set_completion_note_by_id(&day_id, &first_id, "第一段")
        .unwrap();
    assert_eq!(
        state.day.as_ref().unwrap().ledger[1]
            .completion_note
            .as_deref(),
        Some("第一段")
    );
    assert_eq!(state.day.as_ref().unwrap().ledger[0].completion_note, None);

    let before = json(&state);
    for (day_id, entry_id) in [
        ("", first_id.as_str()),
        (day_id.as_str(), ""),
        (day_id.as_str(), "1000"),
    ] {
        assert!(state
            .set_completion_note_by_id(day_id, entry_id, "不得写入")
            .is_err());
    }
    assert!(state.delete_history_day_by_id("").is_err());
    assert_eq!(json(&state), before);
    assert!(state
        .set_completion_note_by_id(&day_id, &first_id, &"字".repeat(2_001))
        .is_err());
    assert!(state
        .set_completion_note_by_id(&day_id, &first_id, "有\0空字符")
        .is_err());
    assert_eq!(json(&state), before);
}

#[test]
fn duplicate_or_missing_sync_ids_are_rejected_without_mutation() {
    let (mut state, _) = archived(1_000, A);
    let day_id = state.history[0].day.sync_id.clone();
    let entry_id = state.history[0].day.ledger[0].sync_id.clone();
    state.history.push(state.history[0].clone());
    let before = json(&state);
    assert!(state.delete_history_day_by_id(&day_id).is_err());
    assert!(state
        .set_completion_note_by_id(&day_id, &entry_id, "不应修改")
        .is_err());
    assert_eq!(json(&state), before);

    state.history.pop();
    state.history[0].day.sync_id.clear();
    state.history[0].day.ledger[0].sync_id.clear();
    let before = json(&state);
    assert!(state.delete_history_day_by_id(&day_id).is_err());
    assert!(state
        .set_completion_note_by_id(&day_id, &entry_id, "不应退回时间戳")
        .is_err());
    assert_eq!(json(&state), before);
}

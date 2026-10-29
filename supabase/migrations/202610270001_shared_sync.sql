-- 坐功共享文档协议 v1。客户端只调用两个 RPC，不能直接写表或指定其他账号。
-- 私有 schema 不加入 Supabase 的 exposed schemas；迁移由数据库管理员执行。
begin;

create schema if not exists sitz_sync_private;
revoke all on schema sitz_sync_private from public, anon, authenticated;

create table if not exists sitz_sync_private.documents (
    user_id uuid primary key references auth.users(id) on delete cascade,
    revision bigint not null default 0 check (revision between 0 and 9007199254740991),
    document jsonb,
    updated_at timestamptz not null default pg_catalog.now(),
    check ((revision = 0 and document is null) or (revision > 0 and document is not null))
);

-- 不为操作记录设自动过期：旧设备重试时也必须认得原操作，不能再次执行。
-- 冲突请求也保存摘要，保证同一 operation_id 不能换内容、设备或基线后再用。
create table if not exists sitz_sync_private.operations (
    user_id uuid not null references sitz_sync_private.documents(user_id) on delete cascade,
    operation_id text not null check (operation_id ~ '^[0-9a-f]{32}$'),
    device_id text not null check (device_id ~ '^[0-9a-f]{32}$'),
    expected_revision bigint not null check (expected_revision between 0 and 9007199254740991),
    document_sha256 bytea not null check (pg_catalog.octet_length(document_sha256) = 32),
    committed_revision bigint check (committed_revision between 1 and 9007199254740991),
    created_at timestamptz not null default pg_catalog.now(),
    primary key (user_id, operation_id)
);

alter table sitz_sync_private.documents enable row level security;
alter table sitz_sync_private.documents force row level security;
alter table sitz_sync_private.operations enable row level security;
alter table sitz_sync_private.operations force row level security;

drop policy if exists own_document on sitz_sync_private.documents;
create policy own_document on sitz_sync_private.documents
    using ((select auth.uid()) = user_id)
    with check ((select auth.uid()) = user_id);
drop policy if exists own_operations on sitz_sync_private.operations;
create policy own_operations on sitz_sync_private.operations
    using ((select auth.uid()) = user_id)
    with check ((select auth.uid()) = user_id);

revoke all on sitz_sync_private.documents, sitz_sync_private.operations from public, anon, authenticated;

-- 校验函数只接收协议常量作为边界；异常不回显任何用户内容。
create or replace function sitz_sync_private.assert_object(p_value jsonb, p_keys text[])
returns void language plpgsql immutable set search_path = '' as $$
begin
    if p_value is null or pg_catalog.jsonb_typeof(p_value) <> 'object' then
        raise exception using errcode = '22023', message = '同步文档对象格式无效';
    end if;
    if not (p_value ?& p_keys)
       or (select pg_catalog.count(*) from pg_catalog.jsonb_object_keys(p_value)) <> pg_catalog.cardinality(p_keys) then
        raise exception using errcode = '22023', message = '同步文档包含缺失或未知字段';
    end if;
end;
$$;

create or replace function sitz_sync_private.assert_array(p_value jsonb, p_max integer, p_min integer default 0)
returns void language plpgsql immutable set search_path = '' as $$
begin
    if p_value is null or pg_catalog.jsonb_typeof(p_value) <> 'array' then
        raise exception using errcode = '22023', message = '同步文档列表格式无效';
    end if;
    if pg_catalog.jsonb_array_length(p_value) not between p_min and p_max then
        raise exception using errcode = '22023', message = '同步文档列表数量超限';
    end if;
end;
$$;

create or replace function sitz_sync_private.assert_text(p_value jsonb, p_max integer, p_nonempty boolean default false, p_multiline boolean default false)
returns text language plpgsql immutable set search_path = '' as $$
declare v_text text;
begin
    if p_value is null or pg_catalog.jsonb_typeof(p_value) <> 'string' then
        raise exception using errcode = '22023', message = '同步文档文本格式无效';
    end if;
    v_text := p_value #>> '{}';
    if pg_catalog.char_length(v_text) > p_max
       or (p_nonempty and v_text ~ '^[[:space:]]*$')
       or (case when p_multiline then pg_catalog.translate(v_text, E'\n\r\t', '') else v_text end) ~ '[[:cntrl:]]' then
        raise exception using errcode = '22023', message = '同步文档文本长度或字符无效';
    end if;
    return v_text;
end;
$$;

create or replace function sitz_sync_private.assert_integer(p_value jsonb, p_min bigint, p_max bigint)
returns bigint language plpgsql immutable set search_path = '' as $$
declare v_text text; v_number numeric;
begin
    if p_value is null or pg_catalog.jsonb_typeof(p_value) <> 'number' then
        raise exception using errcode = '22023', message = '同步文档整数格式无效';
    end if;
    v_text := p_value::text;
    if pg_catalog.length(v_text) > 20 or v_text !~ '^-?(0|[1-9][0-9]*)$' then
        raise exception using errcode = '22023', message = '同步文档整数格式无效';
    end if;
    v_number := v_text::numeric;
    if v_number < p_min or v_number > p_max then
        raise exception using errcode = '22023', message = '同步文档数值超限';
    end if;
    return v_number::bigint;
end;
$$;

create or replace function sitz_sync_private.assert_id(p_value jsonb, p_empty boolean default false)
returns text language plpgsql immutable set search_path = '' as $$
declare v_text text;
begin
    v_text := sitz_sync_private.assert_text(p_value, 64, not p_empty);
    if not (p_empty and v_text = '') and v_text !~ '^[A-Za-z0-9_.:-]{1,64}$' then
        raise exception using errcode = '22023', message = '同步文档项目或计划标识无效';
    end if;
    return v_text;
end;
$$;

create or replace function sitz_sync_private.assert_sync_id(p_value jsonb)
returns text language plpgsql immutable set search_path = '' as $$
declare v_text text;
begin
    v_text := sitz_sync_private.assert_text(p_value, 128, true);
    if v_text !~ '^[0-9a-f]{32}-[a-z0-9-]+$' then
        raise exception using errcode = '22023', message = '同步记录标识无效';
    end if;
    return v_text;
end;
$$;

create or replace function sitz_sync_private.assert_tasks(p_tasks jsonb)
returns void language plpgsql immutable set search_path = '' as $$
declare v_task jsonb;
begin
    perform sitz_sync_private.assert_array(p_tasks, 50);
    for v_task in select value from pg_catalog.jsonb_array_elements(p_tasks) loop
        perform sitz_sync_private.assert_object(v_task, array['text', 'done']);
        perform sitz_sync_private.assert_text(v_task->'text', 500, true, true);
        if pg_catalog.jsonb_typeof(v_task->'done') <> 'boolean' then
            raise exception using errcode = '22023', message = '同步任务完成状态无效';
        end if;
    end loop;
end;
$$;

create or replace function sitz_sync_private.assert_preferences(p_prefs jsonb)
returns void language plpgsql immutable set search_path = '' as $$
declare
    v_item jsonb; v_quota jsonb; v_id text; v_ref text;
    v_categories text[] := array[]::text[];
    v_profiles text[] := array[]::text[];
    v_quotas text[]; v_has_quota boolean; v_minutes bigint;
begin
    perform sitz_sync_private.assert_object(p_prefs, array['categories', 'profiles', 'break_minutes', 'uniform_block_minutes', 'default_profile_id']);
    perform sitz_sync_private.assert_integer(p_prefs->'break_minutes', 0, 120);
    perform sitz_sync_private.assert_integer(p_prefs->'uniform_block_minutes', 0, 180);
    perform sitz_sync_private.assert_array(p_prefs->'categories', 64, 1);
    perform sitz_sync_private.assert_array(p_prefs->'profiles', 16, 1);
    for v_item in select value from pg_catalog.jsonb_array_elements(p_prefs->'categories') loop
        perform sitz_sync_private.assert_object(v_item, array['id', 'name', 'short_name', 'default_block_minutes', 'icon', 'role', 'block_rationale']);
        v_id := sitz_sync_private.assert_id(v_item->'id');
        if v_id = any(v_categories) then
            raise exception using errcode = '22023', message = '同步项目标识重复';
        end if;
        v_categories := pg_catalog.array_append(v_categories, v_id);
        perform sitz_sync_private.assert_text(v_item->'name', 80, true);
        perform sitz_sync_private.assert_text(v_item->'short_name', 32);
        perform sitz_sync_private.assert_text(v_item->'icon', 64);
        perform sitz_sync_private.assert_text(v_item->'role', 64);
        perform sitz_sync_private.assert_text(v_item->'block_rationale', 1000, false, true);
        perform sitz_sync_private.assert_integer(v_item->'default_block_minutes', 1, 180);
    end loop;
    for v_item in select value from pg_catalog.jsonb_array_elements(p_prefs->'profiles') loop
        perform sitz_sync_private.assert_object(v_item, array['id', 'name', 'subtitle', 'quotas']);
        v_id := sitz_sync_private.assert_id(v_item->'id');
        if v_id = any(v_profiles) then
            raise exception using errcode = '22023', message = '同步计划标识重复';
        end if;
        v_profiles := pg_catalog.array_append(v_profiles, v_id);
        perform sitz_sync_private.assert_text(v_item->'name', 80, true);
        perform sitz_sync_private.assert_text(v_item->'subtitle', 1000, false, true);
        perform sitz_sync_private.assert_array(v_item->'quotas', 64, 1);
        v_quotas := array[]::text[];
        v_has_quota := false;
        for v_quota in select value from pg_catalog.jsonb_array_elements(v_item->'quotas') loop
            perform sitz_sync_private.assert_object(v_quota, array['category', 'minutes']);
            v_ref := sitz_sync_private.assert_id(v_quota->'category');
            if not (v_ref = any(v_categories)) or v_ref = any(v_quotas) then
                raise exception using errcode = '22023', message = '同步计划项目引用无效';
            end if;
            v_quotas := pg_catalog.array_append(v_quotas, v_ref);
            -- 先单独校验，不能放在 OR 右侧；已有正配额时短路会漏掉后续坏数值。
            v_minutes := sitz_sync_private.assert_integer(v_quota->'minutes', 0, 1440);
            v_has_quota := v_has_quota or v_minutes > 0;
        end loop;
        if not v_has_quota then
            raise exception using errcode = '22023', message = '同步计划至少需要一个正配额';
        end if;
    end loop;
    v_ref := sitz_sync_private.assert_id(p_prefs->'default_profile_id', true);
    if v_ref <> '' and not (v_ref = any(v_profiles)) then
        raise exception using errcode = '22023', message = '同步默认计划不存在';
    end if;
end;
$$;

-- 返回该学习日的记录标识，外层再检查跨学习日的重复。
create or replace function sitz_sync_private.assert_day(p_day jsonb, p_archived boolean default false)
returns text[] language plpgsql immutable set search_path = '' as $$
declare
    v_item jsonb; v_timer jsonb; v_id text; v_ref text; v_key text;
    v_categories text[] := array[]::text[];
    v_ids text[] := array[]::text[];
    v_start bigint; v_end bigint; v_total bigint;
    v_totals jsonb := '{}'::jsonb;
    v_open boolean := false; v_pause_index integer := 0;
begin
    perform sitz_sync_private.assert_object(p_day, array[
        'sync_id', 'profile_name', 'profile_id', 'started_at', 'categories', 'timer', 'ledger',
        'seated_seconds', 'paused_seconds', 'suspend_seconds', 'cups', 'seated_since_water',
        'seated_since_relief', 'paused_without_block', 'break_until', 'pauses', 'rests'
    ]);
    v_ids := pg_catalog.array_append(v_ids, sitz_sync_private.assert_sync_id(p_day->'sync_id'));
    perform sitz_sync_private.assert_text(p_day->'profile_name', 80, true);
    perform sitz_sync_private.assert_id(p_day->'profile_id', true);
    perform sitz_sync_private.assert_integer(p_day->'started_at', 0, 253402300799);
    foreach v_key in array array['seated_seconds', 'paused_seconds', 'suspend_seconds', 'cups'] loop
        perform sitz_sync_private.assert_integer(p_day->v_key, 0, 34560000);
    end loop;
    if (p_day->>'suspend_seconds')::bigint > (p_day->>'paused_seconds')::bigint then
        raise exception using errcode = '22023', message = '同步挂起时长超过暂停时长';
    end if;
    foreach v_key in array array['seated_since_water', 'seated_since_relief', 'paused_without_block'] loop
        perform sitz_sync_private.assert_integer(p_day->v_key, 0, 0);
    end loop;
    if p_day->'break_until' <> 'null'::jsonb then
        perform sitz_sync_private.assert_integer(p_day->'break_until', 0, 253402300799);
    end if;
    perform sitz_sync_private.assert_array(p_day->'categories', 128, 1);
    for v_item in select value from pg_catalog.jsonb_array_elements(p_day->'categories') loop
        perform sitz_sync_private.assert_object(v_item, array['id', 'name', 'quota_minutes', 'accepted_seconds']);
        v_id := sitz_sync_private.assert_id(v_item->'id');
        if v_id = any(v_categories) then
            raise exception using errcode = '22023', message = '同步学习日项目重复';
        end if;
        v_categories := pg_catalog.array_append(v_categories, v_id);
        perform sitz_sync_private.assert_text(v_item->'name', 80, true);
        perform sitz_sync_private.assert_integer(v_item->'quota_minutes', 0, 1440);
        perform sitz_sync_private.assert_integer(v_item->'accepted_seconds', 0, 34560000);
    end loop;
    v_timer := p_day->'timer';
    if v_timer <> 'null'::jsonb then
        if p_archived then
            raise exception using errcode = '22023', message = '同步归档仍包含运行中的计时';
        end if;
        perform sitz_sync_private.assert_object(v_timer, array['sync_id', 'category', 'total_seconds', 'elapsed_seconds', 'tasks', 'started_at', 'break_minutes']);
        v_ids := pg_catalog.array_append(v_ids, sitz_sync_private.assert_sync_id(v_timer->'sync_id'));
        v_ref := sitz_sync_private.assert_id(v_timer->'category');
        if not (v_ref = any(v_categories)) then
            raise exception using errcode = '22023', message = '同步计时项目不存在';
        end if;
        v_total := sitz_sync_private.assert_integer(v_timer->'total_seconds', 1, 21600);
        perform sitz_sync_private.assert_integer(v_timer->'elapsed_seconds', 0, v_total);
        perform sitz_sync_private.assert_integer(v_timer->'started_at', 0, 253402300799);
        perform sitz_sync_private.assert_integer(v_timer->'break_minutes', 0, 120);
        perform sitz_sync_private.assert_tasks(v_timer->'tasks');
    end if;
    perform sitz_sync_private.assert_array(p_day->'ledger', 10000);
    for v_item in select value from pg_catalog.jsonb_array_elements(p_day->'ledger') loop
        perform sitz_sync_private.assert_object(v_item, array['sync_id', 'category', 'seconds', 'accepted', 'tasks', 'started_at', 'ended_at', 'completion_note']);
        v_ids := pg_catalog.array_append(v_ids, sitz_sync_private.assert_sync_id(v_item->'sync_id'));
        v_ref := sitz_sync_private.assert_id(v_item->'category');
        if not (v_ref = any(v_categories)) then
            raise exception using errcode = '22023', message = '同步台账项目不存在';
        end if;
        perform sitz_sync_private.assert_integer(v_item->'seconds', 1, 21600);
        if pg_catalog.jsonb_typeof(v_item->'accepted') <> 'boolean' then
            raise exception using errcode = '22023', message = '同步台账状态无效';
        end if;
        if (v_item->>'accepted')::boolean then
            v_totals := pg_catalog.jsonb_set(v_totals, array[v_ref], pg_catalog.to_jsonb(
                coalesce((v_totals->>v_ref)::bigint, 0) + (v_item->>'seconds')::bigint
            ), true);
        end if;
        v_start := sitz_sync_private.assert_integer(v_item->'started_at', 0, 253402300799);
        v_end := sitz_sync_private.assert_integer(v_item->'ended_at', 0, 253402300799);
        if v_end < v_start then
            raise exception using errcode = '22023', message = '同步台账终点早于起点';
        end if;
        if v_item->'completion_note' <> 'null'::jsonb then
            perform sitz_sync_private.assert_text(v_item->'completion_note', 2000, false, true);
        end if;
        perform sitz_sync_private.assert_tasks(v_item->'tasks');
    end loop;
    for v_item in select value from pg_catalog.jsonb_array_elements(p_day->'categories') loop
        if (v_item->>'accepted_seconds')::bigint <> coalesce((v_totals->>(v_item->>'id'))::bigint, 0) then
            raise exception using errcode = '22023', message = '同步进度与专注记录不一致';
        end if;
    end loop;
    perform sitz_sync_private.assert_array(p_day->'pauses', 20000);
    for v_item in select value from pg_catalog.jsonb_array_elements(p_day->'pauses') loop
        v_pause_index := v_pause_index + 1;
        perform sitz_sync_private.assert_object(v_item, array['started_at', 'ended_at', 'auto']);
        v_start := sitz_sync_private.assert_integer(v_item->'started_at', 0, 253402300799);
        if v_item->'ended_at' <> 'null'::jsonb then
            v_end := sitz_sync_private.assert_integer(v_item->'ended_at', 0, 253402300799);
            if v_end < v_start then
                raise exception using errcode = '22023', message = '同步暂停终点早于起点';
            end if;
        else
            if p_archived or v_open or v_pause_index <> pg_catalog.jsonb_array_length(p_day->'pauses') then
                raise exception using errcode = '22023', message = '同步未结束暂停段无效';
            end if;
            v_open := true;
        end if;
        if pg_catalog.jsonb_typeof(v_item->'auto') <> 'boolean' then
            raise exception using errcode = '22023', message = '同步暂停状态无效';
        end if;
    end loop;
    perform sitz_sync_private.assert_array(p_day->'rests', 20000);
    for v_item in select value from pg_catalog.jsonb_array_elements(p_day->'rests') loop
        perform sitz_sync_private.assert_object(v_item, array['started_at', 'ended_at']);
        v_start := sitz_sync_private.assert_integer(v_item->'started_at', 0, 253402300799);
        v_end := sitz_sync_private.assert_integer(v_item->'ended_at', 0, 253402300799);
        if v_end < v_start then
            raise exception using errcode = '22023', message = '同步休息终点早于起点';
        end if;
    end loop;
    if p_day->'break_until' <> 'null'::jsonb
       and (p_archived or v_timer <> 'null'::jsonb or not v_open) then
        raise exception using errcode = '22023', message = '同步休息状态无效';
    end if;
    if not p_archived and v_timer = 'null'::jsonb and not v_open then
        raise exception using errcode = '22023', message = '未计时的学习日必须处于暂停状态';
    end if;
    return v_ids;
end;
$$;

create or replace function sitz_sync_private.assert_document(p_document jsonb)
returns void language plpgsql immutable set search_path = '' as $$
declare
    v_item jsonb; v_ids text[] := array[]::text[];
    v_day_ids text[] := array[]::text[]; v_deleted text[] := array[]::text[];
begin
    if p_document is null or pg_catalog.octet_length(p_document::text) > 2097152 then
        raise exception using errcode = '22023', message = '同步文档超过 2 MiB 或为空';
    end if;
    perform sitz_sync_private.assert_object(p_document, array['schema', 'preferences', 'last_tick', 'day', 'history', 'deleted_day_ids']);
    perform sitz_sync_private.assert_integer(p_document->'schema', 1, 1);
    perform sitz_sync_private.assert_integer(p_document->'last_tick', 0, 253402300799);
    perform sitz_sync_private.assert_preferences(p_document->'preferences');
    if p_document->'day' <> 'null'::jsonb then
        v_ids := v_ids || sitz_sync_private.assert_day(p_document->'day', false);
        v_day_ids := pg_catalog.array_append(v_day_ids, p_document #>> '{day,sync_id}');
    end if;
    perform sitz_sync_private.assert_array(p_document->'history', 1000);
    for v_item in select value from pg_catalog.jsonb_array_elements(p_document->'history') loop
        perform sitz_sync_private.assert_object(v_item, array['day', 'ended_at']);
        v_ids := v_ids || sitz_sync_private.assert_day(v_item->'day', true);
        v_day_ids := pg_catalog.array_append(v_day_ids, v_item #>> '{day,sync_id}');
        if sitz_sync_private.assert_integer(v_item->'ended_at', 0, 253402300799)
           < (v_item #>> '{day,started_at}')::bigint then
            raise exception using errcode = '22023', message = '同步归档终点早于起点';
        end if;
        if v_item #> '{day,timer}' <> 'null'::jsonb then
            raise exception using errcode = '22023', message = '同步归档仍包含运行中的计时';
        end if;
    end loop;
    if (select pg_catalog.count(distinct id) from pg_catalog.unnest(v_ids) as ids(id)) <> pg_catalog.cardinality(v_ids) then
        raise exception using errcode = '22023', message = '同步学习日或专注格标识重复';
    end if;
    perform sitz_sync_private.assert_array(p_document->'deleted_day_ids', 10000);
    for v_item in select value from pg_catalog.jsonb_array_elements(p_document->'deleted_day_ids') loop
        v_deleted := pg_catalog.array_append(v_deleted, sitz_sync_private.assert_sync_id(v_item));
    end loop;
    if (select pg_catalog.count(distinct id) from pg_catalog.unnest(v_deleted) as ids(id)) <> pg_catalog.cardinality(v_deleted)
       or v_deleted && v_day_ids then
        raise exception using errcode = '22023', message = '同步删除标记重复或与现存记录冲突';
    end if;
end;
$$;

create or replace function public.sitz_sync_read(p_known_revision bigint default -1)
returns jsonb language plpgsql security definer set search_path = '' as $$
declare
    v_user uuid := auth.uid();
    v_revision bigint; v_document jsonb;
begin
    if v_user is null then
        raise exception using errcode = '42501', message = '请先登录坐功账号';
    end if;
    if p_known_revision is null or p_known_revision not between -1 and 9007199254740991 then
        raise exception using errcode = '22023', message = '同步版本无效';
    end if;
    select revision, document into v_revision, v_document
      from sitz_sync_private.documents where user_id = v_user;
    v_revision := coalesce(v_revision, 0);
    return pg_catalog.jsonb_build_object(
        'revision', v_revision,
        'document', case when v_revision = p_known_revision then null else v_document end,
        'server_time', pg_catalog.floor(extract(epoch from pg_catalog.clock_timestamp()))::bigint
    );
end;
$$;

create or replace function public.sitz_sync_write(
    p_expected_revision bigint,
    p_operation_id text,
    p_device_id text,
    p_document jsonb
)
returns jsonb language plpgsql security definer set search_path = '' as $$
declare
    v_user uuid := auth.uid();
    v_revision bigint; v_document jsonb; v_digest bytea;
    v_operation sitz_sync_private.operations%rowtype;
    v_committed bigint; v_applied boolean := false;
begin
    if v_user is null then
        raise exception using errcode = '42501', message = '请先登录坐功账号';
    end if;
    if p_expected_revision is null or p_expected_revision not between 0 and 9007199254740991
       or p_operation_id is null or pg_catalog.octet_length(p_operation_id) <> 32 or p_operation_id !~ '^[0-9a-f]{32}$'
       or p_device_id is null or pg_catalog.octet_length(p_device_id) <> 32 or p_device_id !~ '^[0-9a-f]{32}$' then
        raise exception using errcode = '22023', message = '同步操作参数无效';
    end if;
    perform sitz_sync_private.assert_document(p_document);
    v_digest := pg_catalog.sha256(pg_catalog.convert_to(p_document::text, 'UTF8'));

    -- 首次并发创建也由唯一键串行，后续所有判断都在同一账号行锁内完成。
    insert into sitz_sync_private.documents(user_id) values (v_user)
        on conflict (user_id) do nothing;
    select revision, document into strict v_revision, v_document
      from sitz_sync_private.documents where user_id = v_user for update;

    select * into v_operation from sitz_sync_private.operations
      where user_id = v_user and operation_id = p_operation_id;
    if found then
        if v_operation.device_id <> p_device_id
           or v_operation.expected_revision <> p_expected_revision
           or v_operation.document_sha256 <> v_digest then
            raise exception using errcode = '22023', message = '同步操作标识已用于另一份请求';
        end if;
        v_committed := v_operation.committed_revision;
        v_applied := v_committed is not null;
    else
        if v_revision = p_expected_revision then
            if v_revision = 9007199254740991 then
                raise exception using errcode = '22023', message = '同步版本已达到上限';
            end if;
            v_revision := v_revision + 1;
            v_document := p_document;
            v_committed := v_revision;
            v_applied := true;
            update sitz_sync_private.documents set revision = v_revision,
                document = v_document, updated_at = pg_catalog.clock_timestamp()
              where user_id = v_user;
        end if;
        insert into sitz_sync_private.operations(user_id, operation_id, device_id, expected_revision, document_sha256, committed_revision)
            values (v_user, p_operation_id, p_device_id, p_expected_revision, v_digest, v_committed);
    end if;
    return pg_catalog.jsonb_build_object(
        'applied', v_applied,
        'revision', v_revision,
        'committed_revision', v_committed,
        'document', v_document,
        'server_time', pg_catalog.floor(extract(epoch from pg_catalog.clock_timestamp()))::bigint
    );
end;
$$;

revoke all on all functions in schema sitz_sync_private from public, anon, authenticated;
revoke all on function public.sitz_sync_read(bigint) from public, anon, authenticated;
revoke all on function public.sitz_sync_write(bigint, text, text, jsonb) from public, anon, authenticated;
grant execute on function public.sitz_sync_read(bigint) to authenticated;
grant execute on function public.sitz_sync_write(bigint, text, text, jsonb) to authenticated;

comment on function public.sitz_sync_read(bigint) is '坐功协议 v1：只读取当前 auth.uid() 的共享文档；相同版本不重复传正文。';
comment on function public.sitz_sync_write(bigint, text, text, jsonb) is '坐功协议 v1：当前账号行锁内比较版本、去重操作并写入；客户端不能指定 user_id。';

commit;

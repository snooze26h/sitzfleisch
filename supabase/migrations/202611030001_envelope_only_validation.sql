-- 坐功共享文档协议 v1 修订：云端只把住文档外层，逐字段规则只留在客户端。
-- 已经运行过 202610270001 的项目执行本文件即可，不必再补 202611020001；新项目按文件名顺序执行全部迁移。
--
-- 以前同一份文档的结构写在三处：Rust 的数据结构、Rust 的严格解析和这里的 PL/pgSQL。
-- 每改一个字段都要三处同步，再到 SQL Editor 手动执行迁移；2026-11-02 还因数据库区域设置
-- 与客户端不一致，误拒过客户端接受的文本。现在云端只检查：
--   * 账号归属、版本比较与操作去重（sitz_sync_write 原样保留）；
--   * 文档是对象、大小上限、schema；
--   * 顶层与 preferences 的键集合，防止把设备偏好、屏蔽列表或令牌之类误传上来。
-- 类型、长度、标识、配对关系等逐字段规则由客户端的严格解析负责：读到坏文档的设备拒绝应用并提示，
-- 本机数据保留。已部署的 0.15.0 / 0.15.2 客户端调用方式不变，原来能写入的文档这里都接受。
begin;

create or replace function sitz_sync_private.assert_document(p_document jsonb)
returns void language plpgsql immutable set search_path = '' as $$
begin
    -- 2 MiB 的上限由客户端按同一种文本长度计算并留了余量；这里再多放 256 KiB，只拦明显越界的写入。
    if p_document is null or pg_catalog.jsonb_typeof(p_document) <> 'object'
       or pg_catalog.octet_length(p_document::text) > 2359296 then
        raise exception using errcode = '22023', message = '同步文档为空、不是对象或超过上限';
    end if;
    if (select pg_catalog.count(*) from pg_catalog.jsonb_object_keys(p_document)) <> 6
       or not p_document ?& array['schema', 'preferences', 'last_tick', 'day', 'history', 'deleted_day_ids']
       or p_document->'schema' <> '1'::jsonb
       or pg_catalog.jsonb_typeof(p_document->'last_tick') <> 'number'
       or pg_catalog.jsonb_typeof(p_document->'day') not in ('object', 'null')
       or pg_catalog.jsonb_typeof(p_document->'history') <> 'array'
       or pg_catalog.jsonb_typeof(p_document->'deleted_day_ids') <> 'array'
       or pg_catalog.jsonb_typeof(p_document->'preferences') <> 'object' then
        raise exception using errcode = '22023', message = '同步文档结构无效';
    end if;
    -- 单独一步：上面确认 preferences 是对象之后才能取它的键。
    if (select pg_catalog.count(*) from pg_catalog.jsonb_object_keys(p_document->'preferences')) <> 5
       or not (p_document->'preferences') ?& array['categories', 'profiles', 'break_minutes', 'uniform_block_minutes', 'default_profile_id'] then
        raise exception using errcode = '22023', message = '同步文档结构无效';
    end if;
end;
$$;

drop function if exists sitz_sync_private.assert_day(jsonb, boolean);
drop function if exists sitz_sync_private.assert_preferences(jsonb);
drop function if exists sitz_sync_private.assert_tasks(jsonb);
drop function if exists sitz_sync_private.assert_sync_id(jsonb);
drop function if exists sitz_sync_private.assert_id(jsonb, boolean);
drop function if exists sitz_sync_private.assert_integer(jsonb, bigint, bigint);
drop function if exists sitz_sync_private.assert_text(jsonb, integer, boolean, boolean);
drop function if exists sitz_sync_private.assert_array(jsonb, integer, integer);
drop function if exists sitz_sync_private.assert_object(jsonb, text[]);

revoke all on all functions in schema sitz_sync_private from public, anon, authenticated;

commit;

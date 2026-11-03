-- 坐功共享文档协议 v1 修订：文本校验不再依赖数据库的区域设置。
-- 已经运行过 202610270001 的项目执行本文件即可；新项目按文件名顺序执行全部迁移。
-- 旧版 assert_text 用 [[:cntrl:]]/[[:space:]]，范围随 lc_ctype 与排序规则提供者变化：
-- glibc 的 C.UTF-8 把 U+2028/U+2029 判成控制字符，ICU 还包括零宽连接符等格式字符。
-- 客户端接受这些字符，云端却拒绝整份文档。这里改用明确码点，与客户端 Rust 的
-- char::is_control（Unicode Cc）及 char::is_whitespace（White_Space）一致。
begin;

create or replace function sitz_sync_private.assert_text(p_value jsonb, p_max integer, p_nonempty boolean default false, p_multiline boolean default false)
returns text language plpgsql immutable set search_path = '' as $$
declare v_text text;
begin
    if p_value is null or pg_catalog.jsonb_typeof(p_value) <> 'string' then
        raise exception using errcode = '22023', message = '同步文档文本格式无效';
    end if;
    v_text := p_value #>> '{}';
    if pg_catalog.char_length(v_text) > p_max
       or (p_nonempty and v_text ~ E'^[\\u0009-\\u000d\\u0020\\u0085\\u00a0\\u1680\\u2000-\\u200a\\u2028\\u2029\\u202f\\u205f\\u3000]*$')
       or (case when p_multiline then pg_catalog.translate(v_text, E'\n\r\t', '') else v_text end) ~ E'[\\u0001-\\u001f\\u007f-\\u009f]' then
        raise exception using errcode = '22023', message = '同步文档文本长度或字符无效';
    end if;
    return v_text;
end;
$$;

-- create or replace 保留原有权限；这里再收紧一次，单独执行本文件也不会给客户端开放私有函数。
revoke all on function sitz_sync_private.assert_text(jsonb, integer, boolean, boolean) from public, anon, authenticated;

commit;

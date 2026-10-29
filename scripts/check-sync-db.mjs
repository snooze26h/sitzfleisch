// 在内存 PostgreSQL 中执行原始迁移和真实权限/事务函数；不连接用户数据库或发送邮件。
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { test } from "node:test";
import { PGlite } from "@electric-sql/pglite";

const migrationPath = new URL("../supabase/migrations/202610270001_shared_sync.sql", import.meta.url);
const fixturePath = new URL("../supabase/tests/fixtures/document.json", import.meta.url);
const migration = await readFile(migrationPath, "utf8");
const fixture = JSON.parse(await readFile(fixturePath, "utf8"));
const accountA = "00000000-0000-4000-8000-00000000000a";
const accountB = "00000000-0000-4000-8000-00000000000b";
const accountC = "00000000-0000-4000-8000-00000000000c";
const deviceA = "a".repeat(32);
const deviceB = "b".repeat(32);
const operation = (n) => n.toString(16).padStart(32, "0");
const changedDocument = () => {
  const doc = structuredClone(fixture);
  doc.preferences.break_minutes = 5;
  return doc;
};

test("共享数据库 RPC、权限、幂等与输入校验", async (t) => {
  const pg = new PGlite("memory://");
  // 只替代 Supabase 的身份上下文；业务函数、私有表、RLS、GRANT 均来自原迁移。
  await pg.exec(`
    create role anon nologin;
    create role authenticated nologin;
    create schema auth;
    create table auth.users (id uuid primary key);
    create function auth.uid() returns uuid language sql stable as $$
      select nullif(current_setting('request.jwt.claim.sub', true), '')::uuid
    $$;
    grant usage on schema auth to anon, authenticated;
    grant execute on function auth.uid() to anon, authenticated;
  `);
  await pg.query("insert into auth.users(id) values ($1), ($2), ($3)", [accountA, accountB, accountC]);

  const asRole = async (role, account = "") => {
    assert.ok(["admin", "anon", "authenticated"].includes(role));
    await pg.exec("reset role");
    await pg.query("select set_config('request.jwt.claim.sub', $1, false)", [account]);
    if (role !== "admin") await pg.exec(`set role ${role}`);
  };
  const read = async (revision = -1) => (await pg.query("select public.sitz_sync_read($1) as response", [revision])).rows[0].response;
  const write = async (revision, id, document = fixture, device = deviceA) => (
    await pg.query("select public.sitz_sync_write($1, $2, $3, $4::jsonb) as response", [revision, id, device, JSON.stringify(document)])
  ).rows[0].response;
  const denied = async (action, code = "42501") => assert.rejects(action, (error) => error.code === code);
  const operationCount = async (account) => {
    await asRole("admin");
    return Number((await pg.query("select count(*) as n from sitz_sync_private.operations where user_id=$1", [account])).rows[0].n);
  };

  try {
    await t.test("迁移可在 PostgreSQL 中执行，私有表启用并强制 RLS", async () => {
      await pg.exec(migration);
      const tables = (await pg.query(`
        select relname, relrowsecurity, relforcerowsecurity
        from pg_catalog.pg_class c join pg_catalog.pg_namespace n on n.oid=c.relnamespace
        where n.nspname='sitz_sync_private' and c.relkind='r' order by relname
      `)).rows;
      assert.deepEqual(tables, [
        { relname: "documents", relrowsecurity: true, relforcerowsecurity: true },
        { relname: "operations", relrowsecurity: true, relforcerowsecurity: true },
      ]);
    });

    await t.test("匿名角色、缺少登录身份及私有帮助函数均被拒绝", async () => {
      await asRole("anon");
      await denied(() => read());
      await denied(() => write(0, operation(1)));
      await asRole("authenticated");
      await denied(() => read());
      await denied(() => write(0, operation(1)));
      await asRole("authenticated", accountA);
      await denied(() => pg.query("select sitz_sync_private.assert_document($1::jsonb)", [JSON.stringify(fixture)]));
    });

    await t.test("首次写入、已知版本读取和相同操作重试符合协议", async () => {
      await asRole("authenticated", accountA);
      const empty = await read();
      assert.equal(empty.revision, 0);
      assert.equal(empty.document, null);
      assert.ok(Number.isSafeInteger(empty.server_time));
      const first = await write(0, operation(1));
      assert.equal(first.applied, true);
      assert.equal(first.revision, 1);
      assert.equal(first.committed_revision, 1);
      assert.deepEqual(first.document, fixture);
      assert.equal((await read(1)).document, null);
      assert.deepEqual((await read()).document, fixture);
      const retry = await write(0, operation(1));
      assert.equal(retry.applied, true);
      assert.equal(retry.revision, 1);
      assert.equal(retry.committed_revision, 1);
      assert.equal(await operationCount(accountA), 1);
    });

    await t.test("账号 B 看不到 A，相同操作编号可以在不同账号独立使用", async () => {
      await asRole("authenticated", accountB);
      assert.equal((await read()).revision, 0);
      const docB = changedDocument();
      const own = await write(0, operation(1), docB, deviceB);
      assert.equal(own.revision, 1);
      assert.deepEqual((await read()).document, docB);
      await asRole("authenticated", accountA);
      assert.deepEqual((await read()).document, fixture);
      // RPC 不存在 user_id 参数，客户端不能借参数选择另一个账号。
      await denied(() => pg.query("select public.sitz_sync_read(p_user_id => $1::uuid)", [accountB]), "42883");
    });

    await t.test("客户端不能绕过 RPC 直接读写任一私有表", async () => {
      await asRole("authenticated", accountA);
      for (const table of ["documents", "operations"]) {
        await denied(() => pg.query(`select * from sitz_sync_private.${table}`));
        await denied(() => pg.query(`update sitz_sync_private.${table} set user_id=$1`, [accountA]));
        await denied(() => pg.query(`delete from sitz_sync_private.${table}`));
        await denied(() => pg.query(`truncate sitz_sync_private.${table}`));
      }
      await denied(() => pg.query("insert into sitz_sync_private.documents(user_id) values ($1)", [accountC]));
    });

    await t.test("即使误授表权限，RLS 仍隔离其他账号的行与写入", async () => {
      await asRole("admin");
      await pg.exec("grant usage on schema sitz_sync_private to authenticated; grant select, insert, update, delete on sitz_sync_private.documents, sitz_sync_private.operations to authenticated");
      try {
        await asRole("authenticated", accountA);
        assert.deepEqual((await pg.query("select user_id from sitz_sync_private.documents")).rows, [{ user_id: accountA }]);
        assert.ok((await pg.query("select user_id from sitz_sync_private.operations")).rows.every((row) => row.user_id === accountA));
        assert.equal((await pg.query("update sitz_sync_private.documents set revision=42 where user_id=$1 returning user_id", [accountB])).rows.length, 0);
        assert.equal((await pg.query("delete from sitz_sync_private.documents where user_id=$1 returning user_id", [accountB])).rows.length, 0);
        await denied(() => pg.query("insert into sitz_sync_private.documents(user_id) values ($1)", [accountC]));
        await denied(() => pg.query("update sitz_sync_private.documents set user_id=$1 where user_id=$2", [accountC, accountA]));
      } finally {
        await asRole("admin");
        await pg.exec("revoke all on sitz_sync_private.documents, sitz_sync_private.operations from authenticated; revoke all on schema sitz_sync_private from authenticated");
      }
    });

    await t.test("版本冲突返回最新文档，旧成功操作重试返回原提交版本", async () => {
      await asRole("authenticated", accountA);
      const conflict = await write(0, operation(2), changedDocument());
      assert.equal(conflict.applied, false);
      assert.equal(conflict.committed_revision, null);
      assert.equal(conflict.revision, 1);
      assert.deepEqual(conflict.document, fixture);
      const next = await write(1, operation(3), changedDocument());
      assert.equal(next.revision, 2);
      assert.equal(next.committed_revision, 2);
      const retried = await write(0, operation(1));
      assert.equal(retried.applied, true);
      assert.equal(retried.committed_revision, 1);
      assert.equal(retried.revision, 2);
      assert.deepEqual(retried.document, changedDocument());
      const oldConflict = await write(0, operation(2), changedDocument());
      assert.equal(oldConflict.applied, false);
      assert.equal(oldConflict.revision, 2);
    });

    await t.test("操作编号不能换文档、设备或基线后复用，包括先前冲突请求", async () => {
      await asRole("authenticated", accountA);
      await denied(() => write(0, operation(1), changedDocument()), "22023");
      await denied(() => write(0, operation(1), fixture, deviceB), "22023");
      await denied(() => write(1, operation(1)), "22023");
      await denied(() => write(2, operation(2), changedDocument()), "22023");
      assert.equal((await read()).revision, 2);
    });

    await t.test("所有请求参数检查类型、长度和安全整数边界", async () => {
      await asRole("authenticated", accountA);
      for (const revision of [null, -2, "9007199254740992"]) await denied(() => read(revision), "22023");
      for (const revision of [null, -1, "9007199254740992"]) await denied(() => write(revision, operation(8)), "22023");
      for (const id of [null, "", "A".repeat(32), "a".repeat(33), "../".repeat(100)]) await denied(() => write(2, id), "22023");
      for (const device of [null, "", "b".repeat(31), "b".repeat(1024)]) await denied(() => write(2, operation(9), fixture, device), "22023");
    });

    await t.test("文档、嵌套字段和超限输入全部拒绝，失败不修改数据或操作记录", async () => {
      const before = await operationCount(accountA);
      await asRole("authenticated", accountA);
      const invalid = [null, [], {}, { ...fixture, schema: 2 }, { ...fixture, schema: 1.5 }, { ...fixture, last_tick: -1 }, { ...fixture, last_tick: 253402300800 }];
      const bad = (edit) => { const doc = structuredClone(fixture); edit(doc); invalid.push(doc); };
      bad((doc) => { doc.access_token = "synthetic-must-never-be-stored"; });
      bad((doc) => { doc.preferences.blocked_hosts = ["example.invalid"]; });
      bad((doc) => { delete doc.preferences.categories[0].name; });
      bad((doc) => { doc.preferences.categories[0].name = " "; });
      bad((doc) => { doc.preferences.categories[0].name = "测试\u0007"; });
      bad((doc) => { doc.preferences.categories[0].id = "../outside"; });
      bad((doc) => { doc.preferences.categories.push(structuredClone(doc.preferences.categories[0])); });
      bad((doc) => {
        const category = structuredClone(doc.preferences.categories[0]); category.id = "writing";
        doc.preferences.categories.push(category);
        doc.preferences.profiles[0].quotas.push({ category: "writing", minutes: -1 });
      });
      bad((doc) => { doc.preferences.profiles[0].quotas[0].category = "missing"; });
      bad((doc) => { doc.preferences.default_profile_id = "missing"; });
      bad((doc) => { doc.day.sync_id = "a".repeat(129); });
      bad((doc) => { doc.day.timer.sync_id = doc.day.ledger[0].sync_id; });
      bad((doc) => { doc.day.timer.elapsed_seconds = 1801; });
      bad((doc) => { doc.day.timer.total_seconds = "1800"; });
      bad((doc) => { doc.day.timer.break_minutes = -1; });
      bad((doc) => { doc.day.timer.category = "missing"; });
      bad((doc) => { doc.day.seated_since_relief = 1; });
      bad((doc) => { doc.day.cups = 34560001; });
      bad((doc) => { doc.day.suspend_seconds = 61; });
      bad((doc) => { doc.day.categories = []; });
      bad((doc) => { doc.day.categories[0].accepted_seconds = 301; });
      bad((doc) => { doc.day.ledger[0].seconds = 0; });
      bad((doc) => { doc.day.ledger[0].accepted = 1; });
      bad((doc) => { doc.day.ledger[0].ended_at = 1; });
      bad((doc) => { doc.day.ledger[0].tasks[0].done = "true"; });
      bad((doc) => { doc.day.ledger[0].tasks[0].text = " \n\t"; });
      bad((doc) => { doc.day.ledger[0].tasks[0].text = "x".repeat(501); });
      bad((doc) => { doc.day.ledger[0].completion_note = "x".repeat(2001); });
      bad((doc) => { doc.day.ledger[0].completion_note = { refresh_token: "synthetic" }; });
      bad((doc) => { doc.day.pauses[0].auto = null; });
      bad((doc) => { doc.day.pauses.unshift({ started_at: 1700000000, ended_at: null, auto: false }); });
      bad((doc) => { doc.day.timer = null; });
      bad((doc) => { doc.day.break_until = 1700000700; });
      bad((doc) => { doc.day.rests.push({ started_at: 30, ended_at: 20 }); });
      bad((doc) => { doc.deleted_day_ids.push(doc.day.sync_id); });
      bad((doc) => { doc.deleted_day_ids.push(`${deviceA}-d-99`, `${deviceA}-d-99`); });
      bad((doc) => { doc.deleted_day_ids = Array.from({ length: 10001 }, (_, i) => `${deviceA}-d-${i + 100}`); });
      bad((doc) => { doc.history = Array(1001).fill({ day: doc.day, ended_at: doc.last_tick }); });
      bad((doc) => { doc.history = [{ day: structuredClone(doc.day), ended_at: doc.last_tick }]; doc.day = null; });
      bad((doc) => {
        doc.day.timer = null; doc.day.pauses[0].ended_at = null;
        doc.history = [{ day: doc.day, ended_at: doc.last_tick }]; doc.day = null;
      });
      bad((doc) => { doc.preferences.categories[0].block_rationale = "大".repeat(700000); });
      for (let i = 0; i < invalid.length; i++) {
        await denied(() => write(2, operation(100 + i), invalid[i]), "22023");
      }
      assert.equal((await read()).revision, 2);
      assert.deepEqual((await read()).document, changedDocument());
      assert.equal(await operationCount(accountA), before);
    });

    await t.test("同一基线的两笔写入只有一笔成功，失败不会丢掉胜出的文档", async () => {
      await asRole("authenticated", accountC);
      // PGlite 是单连接；这里验证两个同基线请求的 CAS 结果，不冒充多连接锁等待测试。
      const pair = await Promise.all([write(0, operation(1000)), write(0, operation(1001), changedDocument())]);
      assert.equal(pair.filter((value) => value.applied).length, 1);
      assert.equal(pair.filter((value) => !value.applied).length, 1);
      assert.ok(pair.every((value) => value.revision === 1));
      assert.deepEqual(pair[0].document, pair[1].document);
    });

    await t.test("归档、删除标记和空活动日可以写入；同操作键顺序变化不影响幂等", async () => {
      await asRole("authenticated", accountC);
      const doc = structuredClone(fixture);
      doc.day.timer = null;
      doc.history = [{ day: doc.day, ended_at: 1700000600 }];
      doc.day = null;
      doc.deleted_day_ids.push(`${deviceB}-d-123`);
      const applied = await write(1, operation(1002), doc);
      assert.equal(applied.revision, 2);
      const reordered = Object.fromEntries(Object.entries(doc).reverse());
      const retried = await write(1, operation(1002), reordered);
      assert.equal(retried.committed_revision, 2);
      assert.equal(retried.revision, 2);
    });

    await t.test("版本上限明确拒绝，不产生溢出或操作记录", async () => {
      await asRole("admin");
      await pg.query("update sitz_sync_private.documents set revision=9007199254740991 where user_id=$1", [accountC]);
      const before = await operationCount(accountC);
      await asRole("authenticated", accountC);
      await denied(() => write("9007199254740991", operation(1003)), "22023");
      assert.equal((await read()).revision, 9007199254740991);
      assert.equal(await operationCount(accountC), before);
    });

    await t.test("迁移重复执行保留数据与权限，账号删除级联清理私有数据", async () => {
      await asRole("admin");
      await pg.exec(migration);
      await asRole("authenticated", accountA);
      assert.equal((await read()).revision, 2);
      await denied(() => pg.query("select * from sitz_sync_private.documents"));
      await asRole("anon");
      await denied(() => read());
      await asRole("admin");
      await pg.query("delete from auth.users where id=$1", [accountC]);
      assert.equal(Number((await pg.query("select count(*) as n from sitz_sync_private.documents where user_id=$1", [accountC])).rows[0].n), 0);
      assert.equal(await operationCount(accountC), 0);
    });
  } finally {
    await pg.close();
  }
});

// 测试源码来自仓库，路径展示便于追溯；不输出数据库内容或模拟会话。
console.log(`数据库迁移：${fileURLToPath(migrationPath)}`);

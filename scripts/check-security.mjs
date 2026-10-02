// 将此次扫描的构建与安装边界保留为可执行的回归检查；不提权、不写系统配置。
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdtemp, readFile, writeFile, mkdir, rm, copyFile, access } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const root = fileURLToPath(new URL("../", import.meta.url));
for (const name of ["build", "check"]) {
  const workflow = await readFile(join(root, ".github/workflows", `${name}.yml`), "utf8");
  const references = [...workflow.matchAll(/\buses:\s*([^\s#]+)/g)];
  assert(references.length > 0);
  for (const [, reference] of references) assert.match(reference, /^[\w-]+\/[\w-]+@[0-9a-f]{40}$/, `${name}: Action 必须锁定完整提交`);
  assert.match(workflow, /^permissions:\n  contents: read$/m, `${name}: 默认只读`);
  assert.match(workflow, /uses: actions\/checkout@[0-9a-f]{40}[^\n]*\n\s+with:\n\s+persist-credentials: false/, `${name}: checkout 不持久化凭据`);
  if (name === "build") {
    const [build, release] = workflow.split("\n  release:\n");
    assert(!build.includes("contents: write"), "构建任务不得有仓库写权限");
    assert.match(release, /permissions:\n\s+contents: write/, "发布任务显式获得写权限");
    assert.match(release, /if: startsWith\(github.ref, 'refs\/tags\/v'\)/, "发布只在版本标签上运行");
  } else assert(!workflow.includes("contents: write"));
}

const installer = await readFile(join(root, "scripts/install-hosts-helper.sh"), "utf8");
const helper = await readFile(join(root, "scripts/hosts-helper.sh"));
const digest = createHash("sha256").update(helper).digest("hex");
assert(installer.includes(`EXPECTED_SHA256=${digest}`), "安装器摘要必须与仓库助手一致");

if (process.platform === "darwin") {
  const directory = await mkdtemp(join(tmpdir(), "sitzfleisch-installer-test-"));
  try {
    const scripts = join(directory, "scripts");
    await mkdir(scripts);
    const path = join(scripts, "install-hosts-helper.sh");
    await copyFile(join(root, "scripts/install-hosts-helper.sh"), path);
    const source = join(scripts, "hosts-helper.sh");
    await writeFile(source, helper);
    const run = () => spawnSync("/bin/sh", [path, "--check"], { encoding: "utf8" });
    assert.equal(run().status, 0, "真实源码只校验、不安装");
    const marker = join(directory, "executed");
    await writeFile(source, `#!/bin/sh\n/usr/bin/touch '${marker}'\nexit 0\n`);
    const result = run();
    assert.notEqual(result.status, 0, "替换助手必须失败");
    assert.match(result.stderr, /完整性校验失败/);
    await assert.rejects(access(marker), "未验证的助手不能执行，哪怕 --check 会成功");
  } finally { await rm(directory, { recursive: true, force: true }); }
}

console.log("安全边界检查通过：Action 提交锁定、任务权限、助手摘要与篡改拒绝。");

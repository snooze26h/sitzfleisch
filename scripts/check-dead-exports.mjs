// 扫描 src/ 里「导出了但没人引用」的函数与常量。
// tsconfig 的 noUnusedLocals 管不到导出，删掉的功能会在这里留下一地断头。
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";

const ROOT = "src";

function walk(dir) {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    return statSync(path).isDirectory() ? walk(path) : path.endsWith(".ts") ? [path] : [];
  });
}

const files = walk(ROOT);
const sources = new Map(files.map((f) => [f, readFileSync(f, "utf8")]));
const all = [...sources.values()].join("\n");

const dead = [];
for (const [file, text] of sources) {
  for (const m of text.matchAll(/^export (?:async )?function ([A-Za-z_]\w*)|^export const ([A-Z_a-z]\w*)/gm)) {
    const name = m[1] ?? m[2];
    const uses = all.match(new RegExp(`\\b${name}\\b`, "g"))?.length ?? 0;
    if (uses <= 1) dead.push(`${file}: ${name}`);
  }
}

if (dead.length) {
  console.error(`导出了但没人用（${dead.length} 个）——删掉，或者说明为什么留着：`);
  for (const line of dead) console.error(`  ${line}`);
  process.exit(1);
}
console.log(`导出检查通过：${files.length} 个文件，没有无人引用的导出。`);

#!/bin/sh
# 坐功的 hosts 安装助手。装到系统目录后由 sudoers 免密调用，让开工与收工不再弹密码框。
#
# 两条设计约束，都是为了把「免密」的代价压到最小：
#   1. 内容走标准输入而不是参数。除 --check 外一律拒收参数，安装脚本也把 sudoers
#      规则收紧到「不带参数」——调用方无法通过换参数改变它的行为。
#   2. 写入前校验内容只能是 hosts 记录。免密意味着这台机器上以该用户身份运行的任何
#      程序都能调用它，所以最坏情况必须被限制在「改 hosts 记录」，而不是任意 root 写入。
#
# 该文件必须 root 拥有、仅 root 可写（安装脚本按 0755 root:wheel 就位）。
# 用户可写就等于把 root 送出去了：任何人改掉脚本内容即可免密执行任意命令。
set -eu
PATH=/usr/bin:/bin:/usr/sbin:/sbin
export PATH

TARGET=/etc/hosts
MAX_BYTES=65536

# --check 只校验标准输入、不写任何文件，供测试在无 root 环境下验证这道防线。
CHECK_ONLY=no
if [ "${1:-}" = "--check" ]; then
  CHECK_ONLY=yes
  shift
fi
if [ "$#" -ne 0 ]; then
  echo "用法：$0 [--check] < 新的 hosts 内容" >&2
  exit 2
fi

umask 022
staged=$(mktemp "/tmp/sitzfleisch-hosts.XXXXXX")
trap 'rm -f "$staged"' EXIT INT TERM
# 最多读取上限加一字节；不能先把无界标准输入全部落盘，再检查大小。
dd bs=1 count="$((MAX_BYTES + 1))" > "$staged" 2>/dev/null

size=$(wc -c < "$staged" | tr -d ' ')
if [ "$size" -eq 0 ]; then
  echo "拒绝写入：内容为空。" >&2
  exit 1
fi
if [ "$size" -gt "$MAX_BYTES" ]; then
  echo "拒绝写入：内容 ${size} 字节，超过上限 ${MAX_BYTES}。" >&2
  exit 1
fi

# 只放行空行、注释行，以及「地址 + 主机名」的记录行。
if ! /usr/bin/awk '
  function ipv4(s, a, n, i) {
    n = split(s, a, ".")
    if (n != 4) return 0
    for (i = 1; i <= n; i++) if (a[i] !~ /^[0-9]+$/ || length(a[i]) > 3 || a[i] + 0 > 255) return 0
    return 1
  }
  function address(s, a, n, i, groups, compressed, zones) {
    if (ipv4(s)) return 1
    zones = split(s, a, "%")
    if (zones > 2 || (zones == 2 && a[2] !~ /^[A-Za-z0-9_.-]+$/)) return 0
    s = a[1]
    if (s !~ /:/ || s ~ /:::/ || (s ~ /^:/ && s !~ /^::/) || (s ~ /:$/ && s !~ /::$/)) return 0
    compressed = index(s, "::") > 0
    n = split(s, a, "::")
    if (n > 2) return 0
    n = split(s, a, ":"); groups = 0
    for (i = 1; i <= n; i++) {
      if (a[i] == "") continue
      if (index(a[i], ".")) {
        if (i != n || !ipv4(a[i])) return 0
        groups += 2
      } else {
        if (a[i] !~ /^[0-9A-Fa-f]+$/ || length(a[i]) > 4) return 0
        groups++
      }
    }
    return compressed ? groups < 8 : groups == 8
  }
  {
    sub(/#.*/, "")
    if ($0 ~ /^[[:blank:]]*$/) next
    if (NF < 2 || !address($1)) exit 1
    for (i = 2; i <= NF; i++) if ($i !~ /^[A-Za-z0-9_][A-Za-z0-9_.-]*$/ || length($i) > 253) exit 1
  }
' "$staged"; then
  echo "拒绝写入：存在不是 hosts 记录的行。" >&2
  exit 1
fi

if [ "$CHECK_ONLY" = yes ]; then
  exit 0
fi

# 用 cp 覆盖内容而不是 mv 换文件：保住 /etc/hosts 原有的 inode、属主与权限。
/bin/cp -f "$staged" "$TARGET"

# 刷新是尽力而为，失败不能把已经写成功的替换报成失败。
# 这里与应用内的授权路径保持一致：整个 mDNSResponder 收掉、由 launchd 拉起，
# 新实例带着空缓存重读 hosts，比 SIGHUP 更确定地丢掉已缓存的派生记录。
/usr/bin/dscacheutil -flushcache >/dev/null 2>&1 || true
/usr/bin/killall mDNSResponder >/dev/null 2>&1 || true
exit 0

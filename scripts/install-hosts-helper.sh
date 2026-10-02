#!/bin/sh
# 一次性安装坐功的 hosts 免密助手（仅 macOS）。装好之后，开学习日与收工不再弹管理员密码框。
#
#   安装： sudo scripts/install-hosts-helper.sh
#   卸载： sudo scripts/install-hosts-helper.sh --uninstall
#   校验： scripts/install-hosts-helper.sh --check（不提权、不安装）
#
# 做两件事：
#   1. 把 hosts-helper.sh 以 root:wheel 0755 装到 /usr/local/libexec/sitzfleisch-hosts-install。
#   2. 在 /etc/sudoers.d/sitzfleisch 写一条只放行这一条命令的免密规则。
#
# 代价要说清楚：装好后，以你的用户身份运行的任何程序都能免密改写 hosts 记录，
# 不再逐次问你。助手会校验内容只能是 hosts 记录，所以影响被限制在域名解析这一层，
# 但这确实比每次输密码弱。不接受就别装，应用会照旧弹授权框。
set -eu
PATH=/usr/bin:/bin:/usr/sbin:/sbin
export PATH

HELPER=/usr/local/libexec/sitzfleisch-hosts-install
SUDOERS=/etc/sudoers.d/sitzfleisch

if [ "$#" -gt 1 ] || { [ "$#" -eq 1 ] && [ "$1" != --uninstall ] && [ "$1" != --check ]; }; then
  echo "用法：sudo $0 [--uninstall]；或 $0 --check" >&2
  exit 2
fi

if [ "$(uname -s)" != Darwin ]; then
  echo "这个脚本只用于 macOS。" >&2
  exit 1
fi

if [ "${1:-}" != --check ] && [ "$(id -u)" -ne 0 ]; then
  echo "需要管理员权限。请运行：sudo $0 $*" >&2
  exit 1
fi

if [ "${1:-}" = "--uninstall" ]; then
  rm -f "$SUDOERS" "$HELPER"
  echo "已卸载：免密规则与助手都已删除，应用恢复为每次弹授权框。"
  exit 0
fi

# 先把外部源码固定在私有目录；摘要通过前绝不执行。自检和安装都使用这份副本，
# 不再重新打开用户可写的源路径，避免两次读取之间被替换。
umask 077
stage_dir=$(/usr/bin/mktemp -d /private/tmp/sitzfleisch-helper.XXXXXX)
trap '/bin/rm -rf "$stage_dir"' EXIT INT TERM
SRC=$(cd "$(dirname "$0")" && pwd)/hosts-helper.sh
verified_helper="$stage_dir/hosts-helper.sh"
EXPECTED_SHA256=f79d59c626e79465a619b56e1042926d85b530c222a25f30b3f97ac99f2a4623
if [ -L "$SRC" ] || [ ! -f "$SRC" ] || ! /bin/cp "$SRC" "$verified_helper"; then
  echo "找不到可信的助手源文件，未安装。" >&2
  exit 1
fi
actual_digest=$(/usr/bin/shasum -a 256 "$verified_helper")
if [ "${actual_digest%% *}" != "$EXPECTED_SHA256" ]; then
  echo "助手完整性校验失败，未执行、未安装。" >&2
  exit 1
fi
if ! printf '127.0.0.1\tlocalhost\n' | /bin/sh "$verified_helper" --check; then
  echo "助手自检未通过，已放弃安装。" >&2
  exit 1
fi
if [ "${1:-}" = --check ]; then
  echo "助手完整性与自检通过，未安装。"
  exit 0
fi

# 免密规则必须绑定到真实使用者，而不是 sudo 之后的 root。
USER_NAME=${SUDO_USER:-}
if [ -z "$USER_NAME" ] || [ "$USER_NAME" = root ]; then
  echo "请用 sudo 从你自己的账号运行，脚本需要据此确定免密的用户。" >&2
  exit 1
fi
case "$USER_NAME" in
  *[!A-Za-z0-9_.-]*|-*|.*) echo "账号短名格式不受支持，未安装。" >&2; exit 1 ;;
esac
if [ "$(id -u "$USER_NAME")" != "${SUDO_UID:-}" ]; then
  echo "sudo 用户身份不一致，未安装。" >&2
  exit 1
fi

# sudoers 信任的不只是脚本，还包括它的父目录；不能把用户可替换的路径当作 root 白名单。
for directory in /usr /usr/local /usr/local/libexec; do
  if [ -L "$directory" ]; then
    echo "助手目录不能是符号链接：$directory" >&2
    exit 1
  fi
  if [ -e "$directory" ]; then
    owner=$(stat -f '%u' "$directory")
    permissions=$(stat -f '%Lp' "$directory")
    if [ ! -d "$directory" ] || [ "$owner" != 0 ] || [ "$((0$permissions & 0022))" -ne 0 ]; then
      echo "助手目录必须由 root 拥有，且不能由组或其他用户写入：$directory" >&2
      exit 1
    fi
  fi
done

install -d -o root -g wheel -m 755 /usr/local/libexec
install -o root -g wheel -m 755 "$verified_helper" "$HELPER"

# 语法错误的 sudoers 片段会让整个 sudo 不可用，所以先用 visudo 校验再就位。
sudoers_tmp="$stage_dir/sudoers"
# 末尾的 "" 把规则收紧到「不带任何参数」。只写命令路径的话 sudoers 允许任意参数，
# 虽然助手自己会拒绝未知参数，但白名单能挡在更前面就挡在更前面。
printf '%s ALL=(root) NOPASSWD: %s ""\n' "$USER_NAME" "$HELPER" > "$sudoers_tmp"
if ! visudo -cf "$sudoers_tmp" >/dev/null; then
  echo "生成的 sudoers 规则未通过校验，已放弃写入。" >&2
  exit 1
fi
install -o root -g wheel -m 440 "$sudoers_tmp" "$SUDOERS"

echo "安装完成。"
echo "  助手：    $HELPER"
echo "  免密规则：$SUDOERS（用户 $USER_NAME）"
echo "下次开学习日与收工不再弹密码框。要还原：sudo $0 --uninstall"

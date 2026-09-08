#!/usr/bin/env bash
#
# wechat-dock-toggle.sh — 微信「托盘 / 窗口切换」修复
#
# 把微信桌面图标变成一个智能入口：
#   - 微信未运行 → 直接启动微信；
#   - 微信已运行 → 从托盘唤出并聚焦主窗口（修复微信驻留托盘后点图标无反应的问题）。
#
# 唤出顺序：
#   1. StatusNotifierItem.Activate（标准托盘协议，按微信进程 ID 定位）；
#   2. GNOME 扩展 Toggler 的 ActivateWeChat（如 toggler@hedgie.tech，存在才调用）；
#   3. X11 兜底：遍历窗口树找微信窗口，raise + 聚焦（需要 python3-xlib）。
#
# 除切换外，还可以把「桌面启动项」装成走这个切换器：
#   apply 会把脚本自身复制到 ~/.local/bin/wechat-dock-toggle，并在
#   ~/.local/share/applications/wechat.desktop 写一份用户级启动项覆盖
#   （Exec 指向切换器），改动前自动备份，restore 可恢复到安装前状态。
#
# 用法：
#   ./wechat-dock-toggle.sh run [--dry-run]       切换微信（默认动作，可省略 run）
#   ./wechat-dock-toggle.sh apply [--dry-run]     安装桌面启动项（先备份）
#   ./wechat-dock-toggle.sh restore [--dry-run] [--backup <时间戳>]
#                                                  从备份恢复（默认最近一次）
#   ./wechat-dock-toggle.sh status                查看当前状态
#   ./wechat-dock-toggle.sh -h | --help
#
# 以当前桌面用户运行，不需要 root。
# 备份目录：~/.local/share/wechat-dock-toggle/backups/<时间戳>/

set -Eeuo pipefail
umask 077

dry_run=false
backup_choice=""
action=run

usage() {
  sed -n '2,30p' "$0"
}

while (($# > 0)); do
  case "$1" in
    run|apply|restore|status)
      action=$1
      ;;
    --dry-run)
      dry_run=true
      ;;
    --backup)
      shift
      if (($# == 0)); then
        printf '--backup 需要一个备份目录名参数\n' >&2
        exit 2
      fi
      backup_choice=$1
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      printf '未知参数：%s\n\n' "$1" >&2
      usage >&2
      exit 2
      ;;
  esac
  shift
done

if [[ ${EUID} -eq 0 ]]; then
  printf '不要用 root / sudo 运行本脚本，请以桌面用户身份运行。\n' >&2
  exit 1
fi

if [[ $(uname -s) != Linux ]]; then
  printf '本脚本仅在 Linux 上运行。\n' >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# 常量
# ---------------------------------------------------------------------------
backup_root="${HOME}/.local/share/wechat-dock-toggle/backups"
desktop_user="${HOME}/.local/share/applications/wechat.desktop"
desktop_system="/usr/share/applications/wechat.desktop"
bin_path="${HOME}/.local/bin/wechat-dock-toggle"

wechat_bin=${WECHAT_BIN:-}
if [[ -z ${wechat_bin} ]]; then
  wechat_bin=$(command -v wechat 2>/dev/null || true)
fi

# ---------------------------------------------------------------------------
# 通用工具
# ---------------------------------------------------------------------------
latest_backup() {
  local dir
  for dir in "${backup_root}"/*/; do
    [[ -d ${dir} ]] || continue
    basename "${dir}"
  done | sort -r | head -n 1
}

backup_current() {
  local stamp=$1
  local dest="${backup_root}/${stamp}"
  printf '备份当前状态到：%s\n' "${dest}"

  if ${dry_run}; then
    [[ -e ${desktop_user} || -L ${desktop_user} ]] && printf '[dry-run] 备份 %s\n' "${desktop_user}"
    return
  fi

  mkdir -p -- "${dest}/before"
  if [[ -e ${desktop_user} || -L ${desktop_user} ]]; then
    cp -a -- "${desktop_user}" "${dest}/before/wechat.desktop"
    printf '  已备份 %s\n' "${desktop_user}"
  else
    printf '  （不存在，跳过）%s\n' "${desktop_user}"
  fi

  {
    printf 'bin_existed_before=%s\n' "$([[ -e ${bin_path} || -L ${bin_path} ]] && echo yes || echo no)"
    printf 'desktop_existed_before=%s\n' "$([[ -e ${desktop_user} || -L ${desktop_user} ]] && echo yes || echo no)"
    printf 'created_at=%s\n' "$(date '+%Y-%m-%d %H:%M:%S')"
  } > "${dest}/state.txt"
  chmod 600 -- "${dest}/state.txt"
}

# ---------------------------------------------------------------------------
# run：切换微信（未运行则启动，已运行则托盘 / 窗口唤出）
# ---------------------------------------------------------------------------
run_toggle() {
  if [[ -z ${wechat_bin} || ! -x ${wechat_bin} ]]; then
    printf '未找到微信可执行文件（期望 /usr/bin/wechat 或在 PATH 中）。\n' >&2
    exit 1
  fi
  if ! command -v python3 >/dev/null 2>&1; then
    printf '需要 python3（Ubuntu 桌面版自带）。\n' >&2
    exit 1
  fi

  local xlib_available=1
  if ! python3 -c 'import Xlib' >/dev/null 2>&1; then
    xlib_available=0
    printf '警告：未安装 python3-xlib，X11 兜底不可用（托盘 / 扩展唤出仍会尝试）。\n' >&2
    printf '       可运行：sudo apt install python3-xlib\n' >&2
  fi

  WECHAT_BIN="${wechat_bin}" \
  WECHAT_XLIB_AVAILABLE="${xlib_available}" \
  WECHAT_DRY_RUN="${dry_run}" \
  python3 - <<'PY'
import os
import subprocess
import sys
import time

DRY_RUN = os.environ.get("WECHAT_DRY_RUN") == "true"
WECHAT_BIN = os.environ.get("WECHAT_BIN", "/usr/bin/wechat")
XLIB_OK = os.environ.get("WECHAT_XLIB_AVAILABLE", "1") == "1"


def log(message):
    prefix = "[dry-run] " if DRY_RUN else ""
    print(f"{prefix}{message}", flush=True)


def wechat_pid():
    result = subprocess.run(
        ["pgrep", "-xo", "wechat"],
        check=False,
        capture_output=True,
        text=True,
    )
    return result.stdout.strip()


def request_tray_activation(pid):
    if DRY_RUN:
        log(f"将通过托盘协议激活微信（StatusNotifierItem，pid={pid}）")
        return
    subprocess.run(
        [
            "gdbus",
            "call",
            "--session",
            "--dest",
            f"org.kde.StatusNotifierItem-{pid}-1",
            "--object-path",
            "/StatusNotifierItem",
            "--method",
            "org.kde.StatusNotifierItem.Activate",
            "0",
            "0",
        ],
        check=False,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )


def request_shell_activation():
    if DRY_RUN:
        log("将通过 GNOME 扩展 Toggler 激活微信（如已安装 toggler@hedgie.tech）")
        return True
    result = subprocess.run(
        [
            "gdbus",
            "call",
            "--session",
            "--dest",
            "org.gnome.Shell.Extensions.Toggler",
            "--object-path",
            "/org/gnome/Shell/Extensions/Toggler",
            "--method",
            "org.gnome.Shell.Extensions.Toggler.ActivateWeChat",
        ],
        check=False,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    return result.returncode == 0


def window_class(window):
    try:
        value = window.get_wm_class()
        return tuple(part.lower() for part in value) if value else ()
    except Exception:
        return ()


def find_wechat_windows(root):
    windows = []
    pending = [root]
    while pending:
        window = pending.pop()
        if "wechat" in window_class(window):
            windows.append(window)
        try:
            pending.extend(window.query_tree().children)
        except Exception:
            pass
    return windows


def activate_existing_window():
    if not XLIB_OK:
        log("X11 兜底不可用（缺 python3-xlib），仅尝试托盘 / 扩展唤出")
        return False
    if DRY_RUN:
        log("将通过 X11 兜底聚焦微信主窗口")
        return True

    from Xlib import X, display, error, protocol

    xdisplay = display.Display()
    root = xdisplay.screen().root
    windows = find_wechat_windows(root)
    if not windows:
        xdisplay.close()
        return False

    # 优先常规的、带标题的顶层窗口，避开助手 / 弹窗。
    def score(window):
        try:
            attributes = window.get_attributes()
            title = window.get_wm_name() or ""
            return (title == "微信", attributes.map_state != X.IsUnmapped, bool(title))
        except error.XError:
            return (False, False, False)

    target = max(windows, key=score)
    try:
        target.map()
        xdisplay.sync()
        time.sleep(0.05)
        target.raise_window()
        target.set_input_focus(X.RevertToParent, X.CurrentTime)
        active_atom = xdisplay.intern_atom("_NET_ACTIVE_WINDOW")
        message = protocol.event.ClientMessage(
            window=target,
            client_type=active_atom,
            data=(32, [2, X.CurrentTime, 0, 0, 0]),
        )
        root.send_event(
            message,
            event_mask=X.SubstructureRedirectMask | X.SubstructureNotifyMask,
        )
        xdisplay.sync()
    except error.XError:
        xdisplay.close()
        return False

    xdisplay.close()
    return True


pid = wechat_pid()
if not pid:
    log(f"微信未运行，将启动：{WECHAT_BIN}")
    if not DRY_RUN:
        os.execv(WECHAT_BIN, [WECHAT_BIN, *sys.argv[1:]])
    sys.exit(0)

log("微信已在运行，尝试从托盘唤出 / 聚焦窗口…")
request_tray_activation(pid)
time.sleep(0.2)
if not request_shell_activation():
    activate_existing_window()
PY
}

# ---------------------------------------------------------------------------
# apply：安装桌面启动项（备份 → 复制切换器 → 写入用户级 wechat.desktop）
# ---------------------------------------------------------------------------
emit_desktop() {
  local bin_path=$1
  local exec_line
  if [[ ${bin_path} == *" "* ]]; then
    exec_line="Exec=\"${bin_path}\" %U"
  else
    exec_line="Exec=${bin_path} %U"
  fi
  local base=""
  if [[ -f ${desktop_user} ]] && ! grep -q 'wechat-dock-toggle' "${desktop_user}" 2>/dev/null; then
    base="${desktop_user}"
  elif [[ -f ${desktop_system} ]]; then
    base="${desktop_system}"
  fi

  if [[ -n ${base} ]]; then
    # 保留原启动项全部字段，只替换 [Desktop Entry] 主段的 Exec。
    awk -v exec_line="${exec_line}" '
      /^\[Desktop Entry\]/ { in_main = 1; print; next }
      /^\[Desktop Action/ { in_main = 0 }
      in_main && /^Exec=/ && !done { print exec_line; done = 1; next }
      { print }
    ' "${base}"
  else
    cat <<EOF
[Desktop Entry]
Name=微信
Name[en_US]=WeChat
Comment=微信桌面版
${exec_line}
StartupNotify=true
StartupWMClass=wechat
Terminal=false
Icon=wechat
Type=Application
Categories=Chat;Network;
EOF
  fi
}

apply_config() {
  if [[ -z ${wechat_bin} || ! -x ${wechat_bin} ]]; then
    printf '未找到微信可执行文件，请先安装微信再安装桌面启动项。\n' >&2
    exit 1
  fi

  printf '安装微信托盘切换桌面启动项\n'
  printf '%s\n' '--------------------------------'
  local stamp
  stamp=$(date +%Y%m%d-%H%M%S-%N)
  backup_current "${stamp}"

  printf '\n写入配置…\n'
  # 1) 复制切换器到 ~/.local/bin（desktop Exec 需要稳定路径，不依赖 App 缓存）
  if ${dry_run}; then
    printf '[dry-run] 复制 %s → %s\n' "$0" "${bin_path}"
  else
    mkdir -p -- "$(dirname -- "${bin_path}")"
    cp -a -- "$0" "${bin_path}"
    chmod 700 -- "${bin_path}"
    printf '  已安装切换器：%s\n' "${bin_path}"
  fi

  # 2) 写入用户级启动项（先写临时文件再原子替换，避免重定向预创建空文件干扰模板选择）
  if ${dry_run}; then
    printf '[dry-run] 写入 %s\n' "${desktop_user}"
  else
    mkdir -p -- "$(dirname -- "${desktop_user}")"
    local tmp_file="${desktop_user}.tmp"
    emit_desktop "${bin_path}" > "${tmp_file}"
    mv -- "${tmp_file}" "${desktop_user}"
    printf '  已写入 %s\n' "${desktop_user}"
  fi

  printf '\n完成。桌面「微信」图标现在会先经过托盘切换器。\n'
  printf '如需撤销：运行 %s restore --backup %s\n' "$0" "${stamp}"
}

# ---------------------------------------------------------------------------
# restore：从备份恢复桌面启动项
# ---------------------------------------------------------------------------
restore_config() {
  local stamp
  if [[ -n ${backup_choice} ]]; then
    stamp=${backup_choice}
  else
    stamp=$(latest_backup || true)
  fi
  if [[ -z ${stamp} || ! -d "${backup_root}/${stamp}/before" ]]; then
    printf '没有可用的备份：%s\n' "${backup_root}/${stamp:-<无>}" >&2
    exit 1
  fi

  local before="${backup_root}/${stamp}/before"
  printf '从备份恢复：%s\n' "${stamp}"
  printf '%s\n' '----------------'

  if [[ -e ${before}/wechat.desktop ]]; then
    if ${dry_run}; then
      printf '[dry-run] 恢复 %s\n' "${desktop_user}"
    else
      mkdir -p -- "$(dirname -- "${desktop_user}")"
      cp -a -- "${before}/wechat.desktop" "${desktop_user}"
      printf '  已恢复 %s\n' "${desktop_user}"
    fi
  else
    if ${dry_run}; then
      printf '[dry-run] 移除（应用前不存在）%s\n' "${desktop_user}"
    else
      rm -f -- "${desktop_user}"
      printf '  已移除（应用前不存在）%s\n' "${desktop_user}"
    fi
  fi

  # 应用前不存在切换器副本时，恢复时一并移除。
  local bin_before
  bin_before=$(sed -n 's/^bin_existed_before=//p' "${backup_root}/${stamp}/state.txt" 2>/dev/null || true)
  if [[ ${bin_before} == "no" && -e ${bin_path} ]]; then
    if ${dry_run}; then
      printf '[dry-run] 移除（应用前不存在）%s\n' "${bin_path}"
    else
      rm -f -- "${bin_path}"
      printf '  已移除（应用前不存在）%s\n' "${bin_path}"
    fi
  fi

  printf '\n恢复完成。桌面启动项已回到安装前状态。\n'
}

# ---------------------------------------------------------------------------
# status：查看当前状态
# ---------------------------------------------------------------------------
show_status() {
  printf '当前微信托盘切换状态\n'
  printf '%s\n' '---------------------'
  printf '会话：%s · %s\n' "${XDG_SESSION_TYPE:-未知}" "${XDG_CURRENT_DESKTOP:-未知}"
  if pgrep -xo wechat >/dev/null 2>&1; then
    printf '微信进程：运行中\n'
  else
    printf '微信进程：未运行\n'
  fi
  if [[ -z ${wechat_bin} ]]; then
    printf '微信可执行文件：未找到\n'
  else
    printf '微信可执行文件：%s\n' "${wechat_bin}"
  fi

  local sys_exec
  sys_exec=$(awk '/^\[Desktop Entry\]/{f=1} /^\[Desktop Action/{f=0} f&&/^Exec=/{print; exit}' "${desktop_system}" 2>/dev/null || true)
  printf '系统级启动项：%s\n' "${sys_exec:-不存在}"

  if [[ -f ${desktop_user} ]]; then
    local user_exec
    user_exec=$(grep -m1 '^Exec=' "${desktop_user}" 2>/dev/null || true)
    printf '用户级启动项：%s\n' "${user_exec:-（无 Exec 行）}"
    if grep -q 'wechat-dock-toggle' "${desktop_user}" 2>/dev/null; then
      printf '  已指向托盘切换器 ✅\n'
    else
      printf '  未指向托盘切换器\n'
    fi
  else
    printf '用户级启动项：不存在（使用系统级）\n'
  fi

  if [[ -x ${bin_path} ]]; then
    printf '切换器副本：%s（存在）\n' "${bin_path}"
  else
    printf '切换器副本：%s（不存在）\n' "${bin_path}"
  fi

  local latest
  latest=$(latest_backup || true)
  printf '最近备份：%s\n' "${latest:-无}"
}

# ---------------------------------------------------------------------------
# 入口
# ---------------------------------------------------------------------------
case "${action}" in
  run)    run_toggle ;;
  apply)  apply_config ;;
  restore) restore_config ;;
  status) show_status ;;
esac

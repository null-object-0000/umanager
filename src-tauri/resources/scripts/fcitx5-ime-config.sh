#!/usr/bin/env bash
#
# fcitx5-ime-config.sh — 应用 / 恢复本机的 Fcitx 5 中文输入法配置
#
# 依据《Ubuntu 系统定制说明》第 4 节与本机实际配置整理。所有改动都在用户目录内，
# 以当前用户运行、不需要 root。有改动前先整体备份，支持 --dry-run 与从备份恢复。
#
# 用法：
#   ./fcitx5-ime-config.sh apply [--dry-run]             应用配置（先备份当前状态）
#   ./fcitx5-ime-config.sh restore [--dry-run] [--backup <时间戳>]
#                                                        从备份恢复（默认最近一次）
#   ./fcitx5-ime-config.sh status                        查看当前输入法状态
#
# 备份目录：~/.local/share/fcitx5-ime-config/backups/<时间戳>/

set -Eeuo pipefail
umask 077

dry_run=false
backup_choice=""

usage() {
  sed -n '2,14p' "$0"
}

while (($# > 0)); do
  case "$1" in
    apply|restore|status)
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

if [[ -z ${action:-} ]]; then
  usage >&2
  exit 2
fi

if [[ ${EUID} -eq 0 ]]; then
  printf '不要用 root / sudo 运行本脚本，请以桌面用户身份运行。\n' >&2
  exit 1
fi

if [[ $(uname -s) != Linux ]]; then
  printf '本脚本仅在 Linux 上运行。\n' >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# 常量：要管理的「HOME 相对路径」集合（最后一项是目录）。
# ---------------------------------------------------------------------------
backup_root="${HOME}/.local/share/fcitx5-ime-config/backups"
theme_rel=".local/share/fcitx5/themes/codex-gnome-light"

managed_rel=(
  ".config/environment.d/input.conf"
  ".config/autostart/org.fcitx.Fcitx5.desktop"
  ".config/fcitx5/profile"
  ".config/fcitx5/conf/classicui.conf"
  "${theme_rel}"
)

# ---------------------------------------------------------------------------
# 输出各配置文件的期望内容（与文档 / 本机实际一致）。
# ---------------------------------------------------------------------------
emit_input_conf() {
  cat <<'EOF'
GTK_IM_MODULE=fcitx
QT_IM_MODULE=fcitx
XMODIFIERS=@im=fcitx
EOF
}

emit_autostart() {
  cat <<'EOF'
[Desktop Entry]
Type=Application
Version=1.0
Name=Fcitx 5
Comment=Start Fcitx 5 input method after logging in
Exec=/usr/bin/fcitx5 -d --replace
Icon=fcitx
Terminal=false
StartupNotify=false
OnlyShowIn=GNOME;
X-GNOME-Autostart-enabled=true
X-GNOME-Autostart-Delay=2
EOF
}

emit_profile() {
  cat <<'EOF'
[Groups/0]
# Group Name
Name=默认
# Layout
Default Layout=us
# Default Input Method
DefaultIM=pinyin

[Groups/0/Items/0]
# Name
Name=keyboard-us
# Layout
Layout=

[Groups/0/Items/1]
# Name
Name=pinyin
# Layout
Layout=

[GroupOrder]
0=默认
EOF
}

emit_classicui_conf() {
  cat <<'EOF'
# Candidate layout
Vertical Candidate List=False
WheelForPaging=True

# Typography and display scaling
Font="Noto Sans CJK SC 16"
MenuFont="Noto Sans CJK SC 14"
TrayFont="Ubuntu Sans Bold 11"
PerScreenDPI=True
ForceWaylandDPI=0

# Appearance
Theme=codex-gnome-light
DarkTheme=codex-gnome-light
UseDarkTheme=False
UseAccentColor=False
EOF
}

emit_theme_conf() {
  cat <<'EOF'
[Metadata]
Name=GNOME Light Modern
Version=1
Author=Codex
Description=Clean light Fcitx5 theme matched to Ubuntu GNOME
ScaleWithDPI=True

[InputPanel]
NormalColor=#2b2b2b
HighlightColor=#ffffff
HighlightCandidateColor=#ffffff
HighlightCandidateLabelColor=#ffffff
CandidateLabelColor=#737373
CandidateCommentColor=#737373
LabelTextSizeFactor=82
CommentTextSizeFactor=82
PageButtonAlignment=Last Candidate

[InputPanel/TextMargin]
Left=9
Right=9
Top=8
Bottom=8

[InputPanel/ContentMargin]
Left=8
Right=8
Top=7
Bottom=7

[InputPanel/Background]
Image=panel.svg

[InputPanel/Background/Margin]
Left=14
Right=14
Top=14
Bottom=14

[InputPanel/Highlight]
Image=highlight.svg

[InputPanel/Highlight/Margin]
Left=10
Right=10
Top=10
Bottom=10

[InputPanel/PrevPage]
Image=prev.svg

[InputPanel/PrevPage/ClickMargin]
Left=7
Right=7
Top=7
Bottom=7

[InputPanel/NextPage]
Image=next.svg

[InputPanel/NextPage/ClickMargin]
Left=7
Right=7
Top=7
Bottom=7

[Menu/Background]
Color=#ffffff
BorderColor=#d8d8d8
BorderWidth=1

[Menu/Background/Margin]
Left=8
Right=8
Top=8
Bottom=8

[Menu/ContentMargin]
Left=4
Right=4
Top=4
Bottom=4

[Menu/Highlight]
Color=#f1f1f1

[Menu/Highlight/Margin]
Left=6
Right=6
Top=6
Bottom=6

[Menu/TextMargin]
Left=8
Right=8
Top=7
Bottom=7
EOF
}

emit_panel_svg() {
  cat <<'EOF'
<svg xmlns="http://www.w3.org/2000/svg" width="48" height="48" viewBox="0 0 48 48">
  <rect x="1" y="1" width="46" height="46" rx="13" fill="#ffffff" stroke="#d8d8d8" stroke-width="1"/>
</svg>
EOF
}

emit_highlight_svg() {
  cat <<'EOF'
<svg xmlns="http://www.w3.org/2000/svg" width="36" height="36" viewBox="0 0 36 36">
  <rect width="36" height="36" rx="10" fill="#e95420"/>
</svg>
EOF
}

emit_next_svg() {
  cat <<'EOF'
<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16">
  <path d="m6 3.5 4.5 4.5L6 12.5" fill="none" stroke="#777777" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/>
</svg>
EOF
}

emit_prev_svg() {
  cat <<'EOF'
<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16">
  <path d="M10 3.5 5.5 8l4.5 4.5" fill="none" stroke="#777777" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/>
</svg>
EOF
}

# ---------------------------------------------------------------------------
# 基础工具
# ---------------------------------------------------------------------------
install_file() {
  local rel=$1
  local path="${HOME}/${rel}"
  if ${dry_run}; then
    printf '[dry-run] 写入 %s\n' "${path}"
    cat >/dev/null
    return
  fi
  mkdir -p -- "$(dirname -- "${path}")"
  cat > "${path}"
}

remove_path() {
  local rel=$1
  local path="${HOME}/${rel}"
  if [[ ! -e ${path} && ! -L ${path} ]]; then
    return
  fi
  if ${dry_run}; then
    printf '[dry-run] 移除 %s\n' "${path}"
    return
  fi
  rm -rf -- "${path}"
}

fcitx5_running() {
  pgrep -x fcitx5 >/dev/null 2>&1
}

kimpanel_enabled() {
  command -v gnome-extensions >/dev/null 2>&1 \
    && gnome-extensions info kimpanel@kde.org >/dev/null 2>&1 \
    && gnome-extensions info kimpanel@kde.org 2>/dev/null | grep -q '已启用: 是\|状态: ACTIVE'
}

restart_fcitx5() {
  if ${dry_run}; then
    printf '[dry-run] 重启 fcitx5（先结束，再 %s -d --replace）\n' "${fcitx5_bin}"
    return
  fi
  pkill -x fcitx5 2>/dev/null || true
  sleep 1
  nohup "${fcitx5_bin}" -d --replace >/dev/null 2>&1 &
  disown || true
  sleep 2
  if fcitx5_running; then
    printf 'fcitx5 已重新启动。\n'
  else
    printf '警告：fcitx5 似乎没有成功启动，请手动运行：fcitx5 -d --replace\n' >&2
  fi
}

check_packages() {
  local missing=()
  local pkg
  for pkg in fcitx5 fcitx5-chinese-addons fcitx5-frontend-all; do
    if ! dpkg-query -W -f='${Status}' "${pkg}" 2>/dev/null | grep -q 'install ok installed'; then
      missing+=("${pkg}")
    fi
  done
  if ((${#missing[@]} > 0)); then
    printf '警告：缺少包 %s。可运行：sudo apt install %s\n' "${missing[*]}" "${missing[*]}" >&2
  else
    printf '依赖包齐全（fcitx5 / chinese-addons / frontend-all）。\n'
  fi
}

# ---------------------------------------------------------------------------
# 备份
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
    for rel in "${managed_rel[@]}"; do
      if [[ -e "${HOME}/${rel}" || -L "${HOME}/${rel}" ]]; then
        printf '[dry-run] 备份 %s\n' "${rel}"
      fi
    done
    return
  fi

  mkdir -p -- "${dest}/before"
  local rel
  for rel in "${managed_rel[@]}"; do
    if [[ -e "${HOME}/${rel}" || -L "${HOME}/${rel}" ]]; then
      mkdir -p -- "$(dirname -- "${dest}/before/${rel}")"
      cp -a -- "${HOME}/${rel}" "${dest}/before/${rel}"
      printf '  已备份 %s\n' "${rel}"
    else
      printf '  （不存在，跳过）%s\n' "${rel}"
    fi
  done

  {
    printf 'kimpanel_enabled=%s\n' "$(kimpanel_enabled && echo yes || echo no)"
    printf 'fcitx5_running=%s\n' "$(fcitx5_running && echo yes || echo no)"
    printf 'created_at=%s\n' "$(date '+%Y-%m-%d %H:%M:%S')"
  } > "${dest}/state.txt"
  chmod 600 -- "${dest}/state.txt"
}

# ---------------------------------------------------------------------------
# 应用
# ---------------------------------------------------------------------------
apply_config() {
  printf '应用 Fcitx 5 输入法配置\n'
  printf '%s\n' '-----------------------'
  check_packages

  local stamp
  stamp=$(date +%Y%m%d-%H%M%S-%N)
  backup_current "${stamp}"

  printf '\n写入配置…\n'
  emit_input_conf     | install_file ".config/environment.d/input.conf"
  emit_autostart      | install_file ".config/autostart/org.fcitx.Fcitx5.desktop"
  emit_profile        | install_file ".config/fcitx5/profile"
  emit_classicui_conf | install_file ".config/fcitx5/conf/classicui.conf"
  emit_theme_conf     | install_file "${theme_rel}/theme.conf"
  emit_panel_svg      | install_file "${theme_rel}/panel.svg"
  emit_highlight_svg  | install_file "${theme_rel}/highlight.svg"
  emit_next_svg       | install_file "${theme_rel}/next.svg"
  emit_prev_svg       | install_file "${theme_rel}/prev.svg"

  if command -v gnome-extensions >/dev/null 2>&1; then
    if ${dry_run}; then
      printf '[dry-run] 启用 kimpanel@kde.org 扩展\n'
    else
      gnome-extensions enable kimpanel@kde.org 2>/dev/null \
        && printf '已启用 kimpanel@kde.org 扩展。\n' \
        || printf '警告：未能启用 kimpanel@kde.org，请确认扩展已安装。\n' >&2
    fi
  else
    printf '未检测到 gnome-extensions，跳过 Kimpanel 扩展开启。\n' >&2
  fi

  restart_fcitx5

  printf '\n完成。配置已写入，备份目录：%s/%s\n' "${backup_root}" "${stamp}"
  printf '如需撤销，运行：%s restore --backup %s\n' "$0" "${stamp}"
}

# ---------------------------------------------------------------------------
# 恢复
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

  printf '从备份恢复：%s\n' "${stamp}"
  printf '%s\n' '----------------'

  local before="${backup_root}/${stamp}/before"
  local rel
  for rel in "${managed_rel[@]}"; do
    if [[ -e "${before}/${rel}" || -L "${before}/${rel}" ]]; then
      if ${dry_run}; then
        printf '[dry-run] 恢复 %s\n' "${rel}"
      else
        remove_path "${rel}"
        mkdir -p -- "$(dirname -- "${HOME}/${rel}")"
        cp -a -- "${before}/${rel}" "${HOME}/${rel}"
        printf '  已恢复 %s\n' "${rel}"
      fi
    else
      if ${dry_run}; then
        printf '[dry-run] 移除（应用前不存在）%s\n' "${rel}"
      else
        remove_path "${rel}"
        printf '  已移除（应用前不存在）%s\n' "${rel}"
      fi
    fi
  done

  local kimpanel_state
  kimpanel_state=$(sed -n 's/^kimpanel_enabled=//p' "${backup_root}/${stamp}/state.txt" 2>/dev/null || true)
  if [[ ${kimpanel_state} == "no" ]] && command -v gnome-extensions >/dev/null 2>&1; then
    if ${dry_run}; then
      printf '[dry-run] 恢复 kimpanel 为禁用\n'
    else
      gnome-extensions disable kimpanel@kde.org 2>/dev/null \
        && printf '已恢复 kimpanel 为禁用。\n' \
        || printf '警告：未能禁用 kimpanel 扩展。\n' >&2
    fi
  fi

  restart_fcitx5
  printf '\n恢复完成。\n'
}

# ---------------------------------------------------------------------------
# 状态
# ---------------------------------------------------------------------------
show_status() {
  printf '当前输入法状态\n'
  printf '%s\n' '--------------'
  printf '会话：%s · %s\n' "${XDG_SESSION_TYPE:-未知}" "${XDG_CURRENT_DESKTOP:-未知}"
  printf 'fcitx5 版本：'
  fcitx5 --version 2>/dev/null | head -n 1 || printf '未安装\n'

  if [[ -f "${HOME}/.config/environment.d/input.conf" ]]; then
    printf '环境变量文件：存在\n'
    grep -q '^GTK_IM_MODULE=fcitx$' "${HOME}/.config/environment.d/input.conf" \
      && grep -q '^QT_IM_MODULE=fcitx$' "${HOME}/.config/environment.d/input.conf" \
      && grep -q '^XMODIFIERS=@im=fcitx$' "${HOME}/.config/environment.d/input.conf" \
      && printf '  内容符合预期（fcitx）。\n' \
      || printf '  内容与预期不一致。\n'
  else
    printf '环境变量文件：不存在\n'
  fi

  [[ -f "${HOME}/.config/autostart/org.fcitx.Fcitx5.desktop" ]] \
    && printf '登录自启动：已配置\n' \
    || printf '登录自启动：未配置\n'

  grep -q '^Theme=codex-gnome-light$' "${HOME}/.config/fcitx5/conf/classicui.conf" 2>/dev/null \
    && printf '候选框主题：codex-gnome-light\n' \
    || printf '候选框主题：不是 codex-gnome-light\n'

  [[ -d "${HOME}/${theme_rel}" ]] \
    && printf '主题目录：存在\n' \
    || printf '主题目录：不存在\n'

  if command -v gnome-extensions >/dev/null 2>&1; then
    kimpanel_enabled && printf 'Kimpanel 扩展：已启用\n' || printf 'Kimpanel 扩展：未启用或未安装\n'
  fi

  fcitx5_running && printf 'fcitx5 进程：运行中\n' || printf 'fcitx5 进程：未运行\n'

  local latest
  latest=$(latest_backup || true)
  printf '最近备份：%s\n' "${latest:-无}"
}

# ---------------------------------------------------------------------------
# 入口
# ---------------------------------------------------------------------------
fcitx5_bin=${FCITX5_BIN:-/usr/bin/fcitx5}

case "${action}" in
  apply)
    apply_config
    ;;
  restore)
    restore_config
    ;;
  status)
    show_status
    ;;
esac

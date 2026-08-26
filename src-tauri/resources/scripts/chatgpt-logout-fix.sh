#!/usr/bin/env bash

set -Eeuo pipefail
umask 077

dry_run=false
reset_ui=false
launch_app=true

usage() {
  cat <<'EOF'
Fix the ChatGPT Desktop logout bug on this Linux machine.

Usage:
  ./fix-chatgpt-logout.sh [--dry-run] [--reset-ui] [--no-launch]

Options:
  --dry-run   Show what would happen without changing anything.
  --reset-ui  Also back up and clear ~/.config/Codex.
              Use this only if clearing the stale login is not enough.
  --no-launch Do not restart ChatGPT after the repair.
  -h, --help  Show this help.

The script never deletes data. Backups are written under:
  ~/.codex/logout-fix-backups/
EOF
}

while (($# > 0)); do
  case "$1" in
    --dry-run)
      dry_run=true
      ;;
    --reset-ui)
      reset_ui=true
      ;;
    --no-launch)
      launch_app=false
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      printf 'Unknown option: %s\n\n' "$1" >&2
      usage >&2
      exit 2
      ;;
  esac
  shift
done

if [[ ${EUID} -eq 0 ]]; then
  printf 'Do not run this script with sudo. Run it as your desktop user.\n' >&2
  exit 1
fi

if [[ $(uname -s) != Linux ]]; then
  printf 'This script is intended for Linux only.\n' >&2
  exit 1
fi

chatgpt_bin=${CHATGPT_BIN:-/usr/lib/chatgpt/ChatGPT}
codex_data_dir=${CODEX_HOME:-"${HOME}/.codex"}
auth_file="${codex_data_dir}/auth.json"
config_root=${XDG_CONFIG_HOME:-"${HOME}/.config"}
ui_data_dir="${config_root}/Codex"
backup_root="${codex_data_dir}/logout-fix-backups"
backup_stamp=$(date +%Y%m%d-%H%M%S-%N)
backup_dir="${backup_root}/${backup_stamp}"
log_file="${TMPDIR:-/tmp}/chatgpt-logout-fix-${UID}.log"
# Optional: set CHATGPT_PROXY_URL to relaunch ChatGPT through a local proxy.
# When unset, the fallback launch uses no proxy.
proxy_url=${CHATGPT_PROXY_URL:-}

if [[ ! -x ${chatgpt_bin} ]]; then
  printf 'ChatGPT executable not found: %s\n' "${chatgpt_bin}" >&2
  exit 1
fi

list_chatgpt_pids() {
  local proc_dir proc_exe
  for proc_dir in /proc/[0-9]*; do
    proc_exe=$(readlink -f "${proc_dir}/exe" 2>/dev/null || true)
    if [[ ${proc_exe} == "${chatgpt_bin}" ]]; then
      printf '%s\n' "${proc_dir##*/}"
    fi
  done
}

stop_chatgpt() {
  local -a process_ids=()
  local -a remaining_ids=()
  local attempt

  mapfile -t process_ids < <(list_chatgpt_pids)
  if ((${#process_ids[@]} == 0)); then
    printf 'ChatGPT is not running.\n'
    return
  fi

  printf 'Found ChatGPT processes: %s\n' "${process_ids[*]}"
  if ${dry_run}; then
    printf '[dry-run] Would send TERM, then KILL only if needed.\n'
    return
  fi

  kill -TERM "${process_ids[@]}" 2>/dev/null || true
  for attempt in {1..10}; do
    mapfile -t remaining_ids < <(list_chatgpt_pids)
    ((${#remaining_ids[@]} == 0)) && break
    sleep 1
  done

  mapfile -t remaining_ids < <(list_chatgpt_pids)
  if ((${#remaining_ids[@]} > 0)); then
    printf 'ChatGPT did not exit cleanly; forcing only these processes: %s\n' \
      "${remaining_ids[*]}"
    kill -KILL "${remaining_ids[@]}" 2>/dev/null || true
    sleep 1
  fi

  mapfile -t remaining_ids < <(list_chatgpt_pids)
  if ((${#remaining_ids[@]} > 0)); then
    printf 'Could not stop ChatGPT processes: %s\n' "${remaining_ids[*]}" >&2
    exit 1
  fi

  printf 'ChatGPT processes stopped.\n'
}

prepare_backup_dir() {
  if ${dry_run}; then
    return
  fi
  mkdir -p -- "${backup_dir}"
  chmod 700 -- "${backup_dir}"
}

backup_stale_auth() {
  if [[ ! -e ${auth_file} ]]; then
    printf 'No stale authentication file found.\n'
    return
  fi
  if [[ -L ${auth_file} || ! -f ${auth_file} ]]; then
    printf 'Refusing unexpected authentication path: %s\n' "${auth_file}" >&2
    exit 1
  fi

  printf 'Authentication file: %s\n' "${auth_file}"
  if ${dry_run}; then
    printf '[dry-run] Would move it to: %s/auth.json\n' "${backup_dir}"
    return
  fi

  prepare_backup_dir
  mv -- "${auth_file}" "${backup_dir}/auth.json"
  chmod 600 -- "${backup_dir}/auth.json"
  printf 'Backed up stale authentication to: %s/auth.json\n' "${backup_dir}"
}

backup_ui_state() {
  if ! ${reset_ui}; then
    return
  fi
  if [[ ! -e ${ui_data_dir} ]]; then
    printf 'No UI state directory found.\n'
    return
  fi
  if [[ -L ${ui_data_dir} || ! -d ${ui_data_dir} ]]; then
    printf 'Refusing unexpected UI state path: %s\n' "${ui_data_dir}" >&2
    exit 1
  fi

  printf 'UI state directory: %s\n' "${ui_data_dir}"
  if ${dry_run}; then
    printf '[dry-run] Would move it to: %s/Codex-user-data\n' "${backup_dir}"
    return
  fi

  prepare_backup_dir
  mv -- "${ui_data_dir}" "${backup_dir}/Codex-user-data"
  printf 'Backed up UI state to: %s/Codex-user-data\n' "${backup_dir}"
}

start_chatgpt() {
  local -a process_ids=()
  local -a launch_args=()

  if ! ${launch_app}; then
    printf 'Skipping application launch.\n'
    return
  fi
  if ${dry_run}; then
    printf '[dry-run] Would launch ChatGPT.\n'
    return
  fi

  if command -v gtk-launch >/dev/null 2>&1; then
    nohup gtk-launch chatgpt >>"${log_file}" 2>&1 </dev/null &
    disown || true
    sleep 4
  fi

  mapfile -t process_ids < <(list_chatgpt_pids)
  if ((${#process_ids[@]} == 0)); then
    printf 'Desktop launcher did not stay running; using direct fallback.\n'
    launch_args=(--class=ChatGPTProxy)
    if [[ -n ${proxy_url} ]]; then
      launch_args+=(--proxy-server="${proxy_url}")
    fi
    nohup "${chatgpt_bin}" "${launch_args[@]}" \
      >>"${log_file}" 2>&1 </dev/null &
    disown || true
    sleep 4
  fi

  mapfile -t process_ids < <(list_chatgpt_pids)
  if ((${#process_ids[@]} == 0)); then
    printf 'ChatGPT did not start. Check: %s\n' "${log_file}" >&2
    exit 1
  fi

  printf 'ChatGPT started. Log: %s\n' "${log_file}"
}

printf 'ChatGPT logout repair\n'
printf '%s\n' '---------------------'
stop_chatgpt
backup_stale_auth
backup_ui_state
start_chatgpt
printf 'Done. The app should now show a clean sign-in flow.\n'

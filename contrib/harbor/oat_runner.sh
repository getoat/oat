#!/bin/sh

set -eu

log_path=""
stats_dir=""
poll_interval_seconds="1"
post_finish_grace_seconds="15"

while [ "$#" -gt 0 ]; do
    case "$1" in
        --log)
            log_path="$2"
            shift 2
            ;;
        --stats-dir)
            stats_dir="$2"
            shift 2
            ;;
        --poll-interval-seconds)
            poll_interval_seconds="$2"
            shift 2
            ;;
        --post-finish-grace-seconds)
            post_finish_grace_seconds="$2"
            shift 2
            ;;
        --)
            shift
            break
            ;;
        *)
            echo "unexpected argument: $1" >&2
            exit 2
            ;;
    esac
done

if [ -z "$log_path" ] || [ -z "$stats_dir" ] || [ "$#" -eq 0 ]; then
    echo "usage: oat_runner.sh --log <path> --stats-dir <dir> -- <command...>" >&2
    exit 2
fi

mkdir -p "$(dirname "$log_path")"

"$@" >"$log_path" 2>&1 &
pid=$!
finished_seen_at=""
forced_completion=0

append_runner_message() {
    printf '[oat-runner] %s\n' "$1" >>"$log_path"
}

session_request_count() {
    path="$1"
    grep -E '"request_count"[[:space:]]*:' "$path" | head -n1 | sed -E 's/.*: *([0-9]+).*/\1/'
}

session_closed_request_count() {
    path="$1"
    completed="$(grep -E '"completed_request_count"[[:space:]]*:' "$path" | head -n1 | sed -E 's/.*: *([0-9]+).*/\1/')"
    failed="$(grep -E '"failed_request_count"[[:space:]]*:' "$path" | head -n1 | sed -E 's/.*: *([0-9]+).*/\1/')"
    interrupted="$(grep -E '"interrupted_request_count"[[:space:]]*:' "$path" | head -n1 | sed -E 's/.*: *([0-9]+).*/\1/')"
    echo $(( ${completed:-0} + ${failed:-0} + ${interrupted:-0} ))
}

latest_terminal_stats() {
    if [ ! -d "$stats_dir" ]; then
        return 1
    fi

    for path in "$stats_dir"/*.json; do
        [ -f "$path" ] || continue
        if grep -Eq '"finished_at_unix_ms"[[:space:]]*:[[:space:]]*[1-9][0-9]*' "$path"; then
            return 0
        fi
        request_count="$(session_request_count "$path")"
        if [ "${request_count:-0}" -gt 0 ] && [ "$(session_closed_request_count "$path")" -ge "$request_count" ]; then
            return 0
        fi
    done

    return 1
}

wait_for_exit() {
    target_pid="$1"
    timeout_seconds="$2"
    deadline=$(( $(date +%s) + timeout_seconds ))
    while [ "$(date +%s)" -lt "$deadline" ]; do
        if ! kill -0 "$target_pid" 2>/dev/null; then
            return 0
        fi
        sleep 1
    done
    return 1
}

while true; do
    if ! kill -0 "$pid" 2>/dev/null; then
        wait "$pid"
        exit_code=$?
        if [ "$forced_completion" -eq 1 ]; then
            exit 0
        fi
        exit "$exit_code"
    fi

    if latest_terminal_stats; then
        now="$(date +%s)"
        if [ -z "$finished_seen_at" ]; then
            finished_seen_at="$now"
            append_runner_message "detected finished Oat stats; waiting for process exit"
        elif [ $(( now - finished_seen_at )) -ge "$post_finish_grace_seconds" ]; then
            forced_completion=1
            append_runner_message "Oat remained alive after stats finalization; sending SIGTERM"
            kill -TERM "$pid" 2>/dev/null || true
            if ! wait_for_exit "$pid" 5; then
                append_runner_message "process did not exit after SIGTERM; sending SIGKILL"
                kill -KILL "$pid" 2>/dev/null || true
                wait_for_exit "$pid" 5 || true
            fi
            wait "$pid" 2>/dev/null || true
            exit 0
        fi
    fi

    sleep "$poll_interval_seconds"
done

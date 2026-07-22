start_guard() {
    local merged="$1"
    local ppid="$2"
    (
        while kill -0 "$ppid" 2>/dev/null; do
            sleep 1
        done
        fusermount -u "$merged" 2>/dev/null || true
    ) &
    GUARD_PID=$!
}

cleanup() {
    cd "$ROOT_DIR" 2>/dev/null || cd / 2>/dev/null || true
    fusermount -u "${MERGED:-}" 2>/dev/null || true
    [ -n "${GUARD_PID:-}" ] && kill "$GUARD_PID" 2>/dev/null || true
    [ -n "${lock_fd:-}" ] && flock -u "$lock_fd" 2>/dev/null || true
    rm -f "$LOCKFILE"
}

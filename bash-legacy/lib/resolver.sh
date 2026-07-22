detect_cycle() {
    local mid="$1"
    local path="$2"
    local did

    CYCLE_STATE[$mid]=1
    for did in ${DEPS[$mid]:-}; do
        [ -z "$did" ] && continue
        if [[ "${CYCLE_STATE[$did]:-}" == "1" ]]; then
            echo "[X] Error: ciclo de dependencias detectado: ${path}${MOD_FOLDER[$did]} -> ${MOD_FOLDER[$did]}"
            HAS_ERRORS=1
            return 1
        fi
        if [[ -z "${CYCLE_STATE[$did]:-}" ]]; then
            detect_cycle "$did" "${path}${MOD_FOLDER[$did]} -> " || return 1
        fi
    done
    CYCLE_STATE[$mid]=2
}

resolve() {
    local mid="$1"
    [[ -n "${VISITED[$mid]:-}" ]] && return
    [[ "${SKIP_DEP[$mid]:-0}" == "1" ]] && return

    VISITED[$mid]=1
    RESOLVED+=("${MOD_FOLDER[$mid]}")

    local dep_ids="${DEPS[$mid]:-}"
    if [ -n "$dep_ids" ]; then
        local dep_list=""
        local did
        for did in $dep_ids; do
            [ -z "$did" ] && continue
            dep_list+="${MOD_ORDER[$did]:-0}|$did"$'\n'
        done
        local sorted
        sorted=$(echo "$dep_list" | LC_ALL=C sort -t'|' -k1,1rn | cut -d'|' -f2 || true)
        for did in $sorted; do
            resolve "$did"
        done
    fi
}

enable_recursive() {
    local did="$1"
    [[ "$did" =~ ^[0-9]+$ ]] || return 1
    if [ "${MOD_ENABLED[$did]:-0}" = "1" ]; then
        return
    fi
    MOD_ENABLED[$did]=1
    sqlite3 "$DB_PATH" "UPDATE mods SET enabled = 1 WHERE id = $did;"
    echo "    [+] Activado: ${MOD_FOLDER[$did]}"
    for sub_did in ${DEPS[$did]:-}; do
        [ -z "$sub_did" ] && continue
        enable_recursive "$sub_did"
    done
}

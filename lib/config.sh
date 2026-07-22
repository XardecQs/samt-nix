parse_toml() {
    local file="$1"
    local key="$2"
    yj -t < "$file" 2>/dev/null | jq -r --arg k "$key" 'if has($k) then .[$k] else empty end' 2>/dev/null
}

toml_bool() {
    case "$1" in
        true)  echo 1 ;;
        false) echo 0 ;;
        *)     echo "Error: valor booleano inválido: '$1'" >&2; exit 1 ;;
    esac
}

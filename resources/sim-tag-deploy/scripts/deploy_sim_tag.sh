#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<'EOF'
Deploy SROS simulation Git tag(s) to remote instances.

Usage:
  deploy_sim_tag.sh <tag> [options]
  deploy_sim_tag.sh --pairs <instance=tag,instance=tag> [options]

Options:
  --pairs <pairs>        deploy different tags per instance, e.g. main=v5.70.0,sim2=v5.71.0
  --repo <path>          sros-core repo path (auto-detected by default)
  --host <host>          remote simulation SSH target (default: std@10.10.21.75)
  --instances <list>     comma-separated instance names for same-tag mode (default: main)
  --mode <fast|build>    fast pulls prebuilt Nexus images; build runs remote build-update (default: fast)
  --build                shorthand for --mode build
  --runtime-tag <tag>    override SROS_RUNTIME_IMAGE_TAG (fast default: same as Git tag)
  --build-name <name>    remote build workspace name (default: sim-release)
  --preset <preset>      CMake preset for runtime build (default: amd64-release)
  --gui                  keep Gazebo GUI running
  --headless             start Gazebo headless (default)
  --smoke                run smoke tests after deploy for each instance
  --no-fetch             do not run git fetch --tags
  --dry-run              print the deployment plan without changing remote hosts
  -h, --help             show this help

Examples:
  # Same tag on the default instance (main), pulling sros-sim-runtime:<tag> from Nexus
  deploy_sim_tag.sh v5.70.0

  # Force the old slow path: sync + remote build-update + start
  deploy_sim_tag.sh v5.70.0 --build

  # Same tag on two instances
  deploy_sim_tag.sh v5.70.0 --instances main,sim2

  # Different tags on two instances
  deploy_sim_tag.sh --pairs main=v5.70.0,sim2=v5.71.0

  # Deploy and then run smoke on both instances
  deploy_sim_tag.sh v5.70.0 --host std@10.10.21.75 --instances main,sim2 --smoke
EOF
}

die() {
    echo "[sim-tag-deploy] ERROR: $*" >&2
    exit 1
}

log() {
    echo "[sim-tag-deploy] $*"
}

trim() {
    printf '%s' "$1" | sed 's/^ *//;s/ *$//'
}

sanitize_name() {
    printf '%s' "$1" | sed 's/[^a-zA-Z0-9_-]/_/g'
}

safe_tag_name() {
    printf '%s' "$1" | sed 's/[^a-zA-Z0-9_.-]/_/g'
}

validate_ssh_host() {
    local value=$1
    if [ -z "$value" ] || [[ "$value" == -* ]] \
        || [[ ! "$value" =~ ^[a-zA-Z0-9_.@:-]+$ ]]; then
        die "invalid ssh host: $value"
    fi
}

validate_cmake_preset() {
    local value=$1
    case "$value" in
        amd64-debug|amd64-release|amd64-profile)
            ;;
        *)
            die "invalid preset: $value"
            ;;
    esac
}

instance_gz_container() {
    local name
    name="$(sanitize_name "$1")"
    printf 'sros-%s-gz-sim' "$name"
}

lock_tag() {
    local lock_file=$1 key=$2
    python3 - "$lock_file" "$key" <<'PY'
import json
import sys

lock_file, key = sys.argv[1], sys.argv[2]
with open(lock_file, encoding='utf-8') as f:
    data = json.load(f)
tag = (data.get('images', {}).get(key) or {}).get('tag')
if tag:
    print(tag)
PY
}

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
skill_dir="$(cd "$script_dir/.." && pwd)"
default_repo="$(cd "$skill_dir/../../.." && pwd)"

tag=""
pairs_csv=""
repo="$default_repo"
host="std@10.10.21.75"
instances_csv="main"
instances_explicit=false
build_name="sim-release"
preset="amd64-release"
deploy_mode="fast"
runtime_tag_override=""
headless=true
run_smoke=false
do_fetch=true
dry_run=false

if [ $# -eq 0 ]; then
    usage
    exit 2
fi

case "$1" in
    -h|--help)
        usage
        exit 0
        ;;
    --*)
        ;;
    *)
        tag="$1"
        shift
        ;;
esac

while [ $# -gt 0 ]; do
    case "$1" in
        --pairs)
            pairs_csv="${2:?--pairs requires instance=tag pairs}"
            shift 2
            ;;
        --repo)
            repo="${2:?--repo requires a path}"
            shift 2
            ;;
        --host)
            host="${2:?--host requires a host}"
            shift 2
            ;;
        --instances)
            instances_csv="${2:?--instances requires comma-separated instance names}"
            instances_explicit=true
            shift 2
            ;;
        --mode)
            deploy_mode="${2:?--mode requires fast or build}"
            shift 2
            ;;
        --build)
            deploy_mode="build"
            shift
            ;;
        --runtime-tag)
            runtime_tag_override="${2:?--runtime-tag requires a value}"
            shift 2
            ;;
        --build-name)
            build_name="${2:?--build-name requires a value}"
            shift 2
            ;;
        --preset)
            preset="${2:?--preset requires a value}"
            shift 2
            ;;
        --gui)
            headless=false
            shift
            ;;
        --headless)
            headless=true
            shift
            ;;
        --smoke)
            run_smoke=true
            shift
            ;;
        --no-fetch)
            do_fetch=false
            shift
            ;;
        --dry-run)
            dry_run=true
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            die "unknown argument: $1"
            ;;
    esac
done

validate_ssh_host "$host"
validate_cmake_preset "$preset"

case "$deploy_mode" in
    fast|build)
        ;;
    *)
        die "invalid --mode: $deploy_mode (expected fast or build)"
        ;;
esac

if [ -n "$pairs_csv" ] && [ -n "$tag" ]; then
    die "use either positional <tag> or --pairs, not both"
fi
if [ -z "$pairs_csv" ] && [ -z "$tag" ]; then
    die "missing tag; use <tag> or --pairs main=<tag>,sim2=<tag>"
fi
if [ -n "$pairs_csv" ] && $instances_explicit; then
    die "--instances is only valid in same-tag mode; use --pairs instance=tag,... for mixed versions"
fi

repo="$(cd "$repo" && pwd)" || die "repo not found: $repo"
git -C "$repo" rev-parse --git-dir >/dev/null 2>&1 || die "not a git repo: $repo"
[ -f "$repo/simulation/scripts/deploy.sh" ] || die "deploy.sh not found under repo: $repo"

if $do_fetch; then
    log "fetch tags in $repo"
    git -C "$repo" fetch --tags
fi

repo_key="$(python3 -c 'import hashlib,sys; print(hashlib.sha1(sys.argv[1].encode()).hexdigest()[:10])' "$repo")"
worktree_root="${TMPDIR:-/tmp}/sros-sim-tag-deploy/${repo_key}"
mkdir -p "$worktree_root"
git -C "$repo" worktree prune >/dev/null 2>&1 || true

prepared_worktree=""
held_locks=()
lock_owner="$$:$RANDOM:$RANDOM"

cleanup_locks() {
    local lock_dir
    for lock_dir in "${held_locks[@]:-}"; do
        release_worktree_lock "$lock_dir"
    done
}
trap cleanup_locks EXIT

remove_held_lock() {
    local lock_dir=$1 kept=() item
    for item in "${held_locks[@]:-}"; do
        [ "$item" = "$lock_dir" ] || kept+=("$item")
    done
    if [ "${#kept[@]}" -eq 0 ]; then
        held_locks=()
    else
        held_locks=("${kept[@]}")
    fi
}

acquire_worktree_lock() {
    local lock_dir=$1 waited=0 owner pid
    while ! mkdir "$lock_dir" 2>/dev/null; do
        if [ -f "$lock_dir/pid" ]; then
            owner="$(cat "$lock_dir/pid" 2>/dev/null || true)"
            pid="${owner%%:*}"
            if [ -n "$pid" ] && ! kill -0 "$pid" 2>/dev/null; then
                if [ "$(cat "$lock_dir/pid" 2>/dev/null || true)" = "$owner" ]; then
                    rm -f "$lock_dir/pid" 2>/dev/null || true
                    rmdir "$lock_dir" 2>/dev/null || true
                fi
                continue
            fi
        fi
        [ "$waited" -lt 120 ] || die "timeout waiting for worktree lock: $lock_dir"
        sleep 1
        waited=$((waited + 1))
    done
    printf '%s\n' "$lock_owner" > "$lock_dir/pid" 2>/dev/null || true
    held_locks+=("$lock_dir")
}

release_worktree_lock() {
    local lock_dir=$1
    if [ "$(cat "$lock_dir/pid" 2>/dev/null || true)" = "$lock_owner" ]; then
        rm -f "$lock_dir/pid" 2>/dev/null || true
        rmdir "$lock_dir" 2>/dev/null || true
    fi
    remove_held_lock "$lock_dir"
}

prepare_worktree() {
    local one_tag=$1 safe_tag worktree lock_dir
    git -C "$repo" rev-parse -q --verify "refs/tags/$one_tag" >/dev/null \
        || die "tag not found: $one_tag"

    safe_tag="$(safe_tag_name "$one_tag")"
    worktree="$worktree_root/$safe_tag"
    lock_dir="$worktree.lock"
    acquire_worktree_lock "$lock_dir"

    if [ -d "$worktree/.git" ] || [ -f "$worktree/.git" ]; then
        log "reuse worktree: $worktree"
        if $do_fetch; then
            git -C "$worktree" fetch --tags >/dev/null 2>&1 || true
        fi
        git -C "$worktree" checkout --force "$one_tag"
    else
        if [ -e "$worktree" ]; then
            die "worktree path exists but is not a git worktree: $worktree"
        fi
        log "create worktree: $worktree"
        git -C "$repo" worktree add --detach "$worktree" "$one_tag"
    fi

    release_worktree_lock "$lock_dir"
    prepared_worktree="$worktree"
}

start_instance() {
    local worktree=$1 instance=$2 gz_tag=$3 rerun_tag=$4 runtime_tag=$5 start_mode
    start_mode="--headless"
    if ! $headless; then
        start_mode="--gui"
    fi

    log "start instance=$instance"
    if $dry_run; then
        printf 'cd %q && GZ_SIM_IMAGE_TAG=%q RERUN_IMAGE_TAG=%q SROS_RUNTIME_IMAGE_TAG=%q ./scripts/deploy.sh remote start %q --instance=%q %s\n' \
            "$worktree/simulation" "$gz_tag" "$rerun_tag" "$runtime_tag" "$host" "$instance" "$start_mode"
    else
        (
            cd "$worktree/simulation"
            GZ_SIM_IMAGE_TAG="$gz_tag" \
            RERUN_IMAGE_TAG="$rerun_tag" \
            SROS_RUNTIME_IMAGE_TAG="$runtime_tag" \
                ./scripts/deploy.sh remote start "$host" --instance="$instance" "$start_mode"
        )
    fi
    disable_vnc_screensaver "$instance"
}

smoke_instance() {
    local worktree=$1 instance=$2
    if ! $run_smoke; then
        return
    fi

    log "run smoke tests instance=$instance"
    if $dry_run; then
        printf 'cd %q && ./scripts/deploy.sh remote sim-test %q smoke --instance=%q\n' \
            "$worktree/simulation" "$host" "$instance"
    else
        (
            cd "$worktree/simulation"
            ./scripts/deploy.sh remote sim-test "$host" smoke --instance="$instance"
        )
    fi
}

build_runtime() {
    local worktree=$1 instance=$2 one_tag=$3
    log "build runtime tag=$one_tag using instance=$instance"
    if $dry_run; then
        printf 'cd %q && ./scripts/deploy.sh remote build-update %q --name=%q --preset=%q --instance=%q\n' \
            "$worktree/simulation" "$host" "$build_name" "$preset" "$instance"
    else
        (
            cd "$worktree/simulation"
            ./scripts/deploy.sh remote build-update "$host" \
                --name="$build_name" \
                --preset="$preset" \
                --instance="$instance"
        )
    fi
}

ensure_build_runtime_tag_resolvable() {
    local worktree=$1 one_tag=$2
    if $dry_run || [ -n "$runtime_tag_override" ] || [ -f "$worktree/simulation/sim-images.lock.json" ]; then
        return
    fi

    die "build mode for legacy tag $one_tag cannot infer runtime tag after build; use fast mode or pass --runtime-tag"
}

disable_vnc_screensaver() {
    local instance=$1 gz_container remote_cmd
    if $headless; then
        return
    fi

    gz_container="$(instance_gz_container "$instance")"
    remote_cmd="for i in \$(seq 1 30); do docker exec ${gz_container} bash -lc 'DISPLAY=:1 xset s off s noblank >/dev/null 2>&1 || exit 1; DISPLAY=:1 xset -dpms >/dev/null 2>&1 || true; DISPLAY=:1 xset s reset >/dev/null 2>&1 || true' && exit 0; sleep 1; done; exit 1"

    log "disable VNC screen saver instance=$instance container=$gz_container"
    if $dry_run; then
        printf 'ssh %q %q\n' "$host" "$remote_cmd"
    else
        ssh -- "$host" "$remote_cmd" \
            || log "warn: failed to disable VNC screen saver for $gz_container"
    fi
}

read_lock_tags() {
    local worktree=$1 one_tag=$2 lock_file lock_source
    lock_file="$worktree/simulation/sim-images.lock.json"
    lock_source="tag worktree"
    if [ ! -f "$lock_file" ]; then
        lock_file="$repo/simulation/sim-images.lock.json"
        lock_source="current repo fallback"
        [ -f "$lock_file" ] || die "missing lock file in tag worktree and current repo: $worktree/simulation/sim-images.lock.json"
        log "warn: tag worktree has no sim-images.lock.json; using current repo lock for gz/rerun tags"
    fi

    gz_tag="$(lock_tag "$lock_file" gz_sim)"
    rerun_tag="$(lock_tag "$lock_file" rerun_viewer)"
    [ -n "$gz_tag" ] || die "missing gz_sim tag in $lock_file"
    [ -n "$rerun_tag" ] || die "missing rerun_viewer tag in $lock_file"

    if [ -n "$runtime_tag_override" ]; then
        runtime_tag="$runtime_tag_override"
    elif [ "$deploy_mode" = "fast" ]; then
        runtime_tag="$one_tag"
    elif $dry_run; then
        runtime_tag="\${SROS_RUNTIME_IMAGE_TAG_FROM_LOCK_AFTER_BUILD_UPDATE}"
    else
        if [ "$lock_source" != "tag worktree" ]; then
            die "build mode cannot infer runtime tag for a legacy tag without sim-images.lock.json; use fast mode or --runtime-tag"
        fi
        runtime_tag="$(lock_tag "$lock_file" sros_runtime)"
        [ -n "$runtime_tag" ] || die "missing sros_runtime tag in $lock_file"
    fi

    log "image tags: gz=$gz_tag rerun=$rerun_tag runtime=$runtime_tag (lock=$lock_source)"
}

if $dry_run; then
    log "dry-run: no remote command will be executed"
fi

if [ -z "$pairs_csv" ]; then
    IFS=',' read -r -a raw_instances <<< "$instances_csv"
    [ "${#raw_instances[@]}" -gt 0 ] || die "no instances parsed from: $instances_csv"

    clean_instances=()
    for raw_instance in "${raw_instances[@]}"; do
        instance="$(sanitize_name "$(trim "$raw_instance")")"
        [ -n "$instance" ] || die "empty instance in list: $instances_csv"
        clean_instances+=("$instance")
    done

    first_instance="${clean_instances[0]}"
    log "mode=same-tag deploy_mode=$deploy_mode tag=$tag host=$host instances=${instances_csv}"
    prepare_worktree "$tag"
    worktree="$prepared_worktree"
    if [ "$deploy_mode" = "build" ]; then
        log "remote build-update is explicit; this is the slow path"
        ensure_build_runtime_tag_resolvable "$worktree" "$tag"
        build_runtime "$worktree" "$first_instance" "$tag"
    else
        log "fast deploy: use prebuilt runtime image tag ${runtime_tag_override:-$tag}"
    fi
    read_lock_tags "$worktree" "$tag"

    for instance in "${clean_instances[@]}"; do
        start_instance "$worktree" "$instance" "$gz_tag" "$rerun_tag" "$runtime_tag"
        smoke_instance "$worktree" "$instance"
    done
else
    IFS=',' read -r -a raw_pairs <<< "$pairs_csv"
    [ "${#raw_pairs[@]}" -gt 0 ] || die "no pairs parsed from: $pairs_csv"

    log "mode=mixed-tags deploy_mode=$deploy_mode host=$host pairs=${pairs_csv}"
    for raw_pair in "${raw_pairs[@]}"; do
        pair="$(trim "$raw_pair")"
        [[ "$pair" == *=* ]] || die "invalid pair (expected instance=tag): $pair"
        instance="$(sanitize_name "$(trim "${pair%%=*}")")"
        one_tag="$(trim "${pair#*=}")"
        [ -n "$instance" ] || die "empty instance in pair: $pair"
        [ -n "$one_tag" ] || die "empty tag in pair: $pair"

        prepare_worktree "$one_tag"
        worktree="$prepared_worktree"
        if [ "$deploy_mode" = "build" ]; then
            log "remote build-update is explicit; this is the slow path"
            ensure_build_runtime_tag_resolvable "$worktree" "$one_tag"
            build_runtime "$worktree" "$instance" "$one_tag"
        else
            log "fast deploy: use prebuilt runtime image tag ${runtime_tag_override:-$one_tag}"
        fi
        read_lock_tags "$worktree" "$one_tag"
        start_instance "$worktree" "$instance" "$gz_tag" "$rerun_tag" "$runtime_tag"
        smoke_instance "$worktree" "$instance"
    done
fi

log "deployment complete"
if $dry_run; then
    printf 'cd <last-worktree>/simulation && ./scripts/deploy.sh remote status %q\n' "$host"
else
    (
        cd "$worktree/simulation"
        ./scripts/deploy.sh remote status "$host"
    )
fi

# install-prereq.sh: the CHECK-ONLY prerequisite include of the Mac installer (kai b935583a (A), cox 850ef045). Sourced by install-mac.sh, never run alone; it has no shebang and no 'set'.
# It LOOKS at this Mac and CHANGES NOTHING: no sudo, no installer, no download, no file, no setting. It makes NO TRUST DECISION (the release check is the shared fetch include's, unchanged).
# What it checks: the install home is under a folder Docker Desktop shares; Docker Desktop is installed and RUNNING; docker compose is version 2 (the override file uses YAML tags that need it); the
# memory Docker Desktop gives its VM is at or above the floor. Every refusal is ONE problem line and ONE fix line (prereq_msg: the installer's msg looks here first) and ends with the exit code of the
# catalogue (EX_MACHINE 2 for the home, EX_PREREQ 3 for Docker Desktop). The Homebrew GNU tools are checked by install-mac.sh itself (its seam block), not here.
# DRAFT FLOORS (ASK-A-MAC, NOT RUN on a Mac): the memory floor is bob's M-1b figure (3.5 GB of limits) and the Compose floor is 2.24.4, the first version that has the YAML tags the override uses (mike MAC-MIKE-01: a major.minor floor of 2.24 let 2.24.0 to 2.24.3 through; the PATCH is compared too).
prereq_floors() { MAC_MEM_FLOOR_MIB=3584; MAC_COMPOSE_FLOOR_MAJOR=2; MAC_COMPOSE_FLOOR_MINOR=24; MAC_COMPOSE_FLOOR_PATCH=4; }   # a function: an included source holds functions only (S5-10), sourcing it runs nothing
prereq_msg() {
  prereq_floors
  pm_again="then run this installer again"
  case $1 in
    home_not_shared) pm_cause="the install home is not under a folder that Docker Desktop shares with its containers${2:+ ($2)}"; pm_fix="choose a --home under /Users, /Volumes, /private, /tmp or /var/folders (the folders Docker Desktop shares by default), or add the folder under Docker Desktop > Settings > Resources > File sharing, $pm_again" ;;
    docker_missing) pm_cause="Docker Desktop is not installed (no docker command was found)"; pm_fix="install Docker Desktop for Mac from docker.com (this installer cannot install it for you), start it, $pm_again" ;;
    docker_down) pm_cause="Docker Desktop is installed but not running (the docker command cannot reach it)"; pm_fix="start Docker Desktop, wait until it says it is running, $pm_again" ;;
    compose_missing) pm_cause="docker compose is not available in this Docker Desktop"; pm_fix="update Docker Desktop (it bundles docker compose version 2), $pm_again" ;;
    compose_form) pm_cause="the docker compose version has a form this installer does not recognise${2:+ ($2)}: a pre-release or a build cannot be placed against $MAC_COMPOSE_FLOOR_MAJOR.$MAC_COMPOSE_FLOOR_MINOR.$MAC_COMPOSE_FLOOR_PATCH"; pm_fix="use the docker compose that comes with a Docker Desktop release (a plain version, or one that ends in -desktop.N), $pm_again" ;;
    compose_old) pm_cause="docker compose is older than version $MAC_COMPOSE_FLOOR_MAJOR.$MAC_COMPOSE_FLOOR_MINOR.$MAC_COMPOSE_FLOOR_PATCH${2:+ (found $2)}"; pm_fix="update Docker Desktop to a version that bundles docker compose $MAC_COMPOSE_FLOOR_MAJOR.$MAC_COMPOSE_FLOOR_MINOR.$MAC_COMPOSE_FLOOR_PATCH or newer, $pm_again" ;;
    docker_memory) pm_cause="Docker Desktop gives its virtual machine too little memory${2:+ ($2)}: the stack needs at least $MAC_MEM_FLOOR_MIB MiB"; pm_fix="raise the memory under Docker Desktop > Settings > Resources, apply and restart it, $pm_again" ;;
    docker_memory_unread) pm_cause="the memory of the Docker Desktop virtual machine could not be read from 'docker info'"; pm_fix="check that 'docker info' works and prints a 'Total Memory' line, $pm_again" ;;
    *) return 1 ;;
  esac
  printf 'problem: %s\n' "$pm_cause" >&2
  printf 'fix: %s\n' "$pm_fix" >&2
}
# prereq_mem_mib TEXT: the memory of 'docker info' (the value of its 'Total Memory:' line, such as 7.656GiB or 7840MiB) as whole MiB on stdout, or -1 for text that is not a number with MiB, GiB or TiB.
prereq_mem_mib() {
  LC_ALL=C awk -v s="$1" 'BEGIN { n = s + 0; u = s; sub(/^[0123456789.]+/, "", u); if (s !~ /^[0123456789]/) m = -1; else if (u == "MiB") m = n; else if (u == "GiB") m = n * 1024; else if (u == "TiB") m = n * 1048576; else m = -1; printf "%d\n", m }'
}
# machine_look: the whole look. Called bare by install-mac.sh (its status ends the shell as it is; every refusal goes through stop, which prints the cause and the fix).
machine_look() {
  prereq_floors
  case $home_dir in /Users/*|/Volumes/*|/private/*|/tmp/*|/var/folders/*) ;; *) stop "$EX_MACHINE" home_not_shared "$home_dir" "the home $home_dir is not under a folder that Docker Desktop shares" ;; esac
  [ -n "$DOCKER_BIN" ] || stop "$EX_PREREQ" docker_missing "" "Docker Desktop is not installed"
  pl_info=$(docker info 2>/dev/null) || stop "$EX_PREREQ" docker_down "" "Docker Desktop is not running"
  pl_cv=$(docker compose version 2>/dev/null) || stop "$EX_PREREQ" compose_missing "" "docker compose is not available"
  pl_raw=${pl_cv##*ersion }; pl_raw=${pl_raw#v}; pl_cvn=${pl_raw%%[!0123456789.]*}
  # the text after the numbers: NONE, or Docker Desktop's own stable form -desktop.<digits>; ANYTHING ELSE (-rc1, -beta.1, +dev, a build tag) is refused: a pre-release of 2.24.4 is OLDER than 2.24.4 and nothing proves what another form is (mike, re-read of 48ad6e0)
  pl_suf=${pl_raw#"$pl_cvn"}
  case $pl_suf in
    "") ;;
    -desktop.*) case ${pl_suf#-desktop.} in ""|*[!0123456789]*) stop "$EX_PREREQ" compose_form "$pl_cv" "the docker compose version has a suffix this installer does not recognise" ;; esac ;;
    *) stop "$EX_PREREQ" compose_form "$pl_cv" "the docker compose version has a suffix this installer does not recognise" ;;
  esac
  case $pl_cvn in ""|.*|*..*) stop "$EX_PREREQ" compose_old "$pl_cv" "the docker compose version could not be read" ;; esac
  pl_maj=${pl_cvn%%.*}; pl_rest=${pl_cvn#*.}; pl_min=${pl_rest%%.*}
  case $pl_maj in ""|*[!0123456789]*) stop "$EX_PREREQ" compose_old "$pl_cvn" "the docker compose version could not be read" ;; esac
  case $pl_min in ""|*[!0123456789]*) pl_min=0 ;; esac
  case $pl_rest in *.*) pl_pat=${pl_rest#*.}; pl_pat=${pl_pat%%.*} ;; *) pl_pat=0 ;; esac   # a missing or unreadable PATCH counts as 0: it fails closed (below the floor)
  case $pl_pat in ""|*[!0123456789]*) pl_pat=0 ;; esac
  pl_below=0
  if [ "$pl_maj" -lt "$MAC_COMPOSE_FLOOR_MAJOR" ]; then pl_below=1
  elif [ "$pl_maj" -eq "$MAC_COMPOSE_FLOOR_MAJOR" ]; then
    if [ "$pl_min" -lt "$MAC_COMPOSE_FLOOR_MINOR" ]; then pl_below=1
    elif [ "$pl_min" -eq "$MAC_COMPOSE_FLOOR_MINOR" ] && [ "$pl_pat" -lt "$MAC_COMPOSE_FLOOR_PATCH" ]; then pl_below=1
    fi
  fi
  if [ "$pl_below" = 1 ]; then
    stop "$EX_PREREQ" compose_old "$pl_cvn" "docker compose $pl_cvn is older than $MAC_COMPOSE_FLOOR_MAJOR.$MAC_COMPOSE_FLOOR_MINOR.$MAC_COMPOSE_FLOOR_PATCH"
  fi
  pl_mem=$(printf '%s\n' "$pl_info" | sed -n 's/^ *Total Memory: *//p' | sed -n 1p)
  pl_mib=$(prereq_mem_mib "$pl_mem") || pl_mib=-1
  case $pl_mib in ""|-*|*[!0123456789]*) stop "$EX_PREREQ" docker_memory_unread "" "the Docker Desktop memory could not be read" ;; esac
  [ "$pl_mib" -ge "$MAC_MEM_FLOOR_MIB" ] || stop "$EX_PREREQ" docker_memory "$pl_mem" "Docker Desktop's virtual machine has $pl_mem, less than $MAC_MEM_FLOOR_MIB MiB"
}

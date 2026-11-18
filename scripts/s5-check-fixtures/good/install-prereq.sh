# Fixture: an INCLUDED source (cox ruling on S5-10): the release builder joins it into install.sh, so it has no shebang and holds functions only.
prereq_demo() {
  case $1 in
    yes) say "demo" ;;
    *) { warn "other"; } ;;
  esac
}
prereq_one() { say one; }
prereq_last() {
  :
}

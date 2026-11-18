import os, sys
out = sys.argv[1]
os.makedirs(out, exist_ok=True)
S = "localhost/cortex-server@sha256:" + "a"*64
D = "docker.io/pgvector/pgvector@sha256:" + "b"*64
def f(v="0.2.1-rc.2", s=S, d=D): return f"CORTEX_VERSION={v}\nCORTEX_SERVER_IMAGE={s}\nCORTEX_DB_IMAGE={d}\n"
C = {}
# ---- files both sides should ACCEPT
C["good-basic"] = f()
C["good-no-final-newline"] = f().rstrip("\n")
C["good-any-order"] = f"CORTEX_DB_IMAGE={D}\nCORTEX_VERSION=1.2.3\nCORTEX_SERVER_IMAGE={S}\n"
C["good-comments-blanks"] = "# a comment\n\n" + f() + "\n# trailing comment\n"
C["good-indented-comment"] = "   # indented\n\t# tabbed\n" + f()
C["good-blank-ws-lines"] = "  \t \n" + f() + " \n\t\n"
C["good-cr-only-line"] = "\r\n" + f()
C["good-comment-with-cr"] = "# crlf comment\r\n" + f()
C["good-nonascii-comment"] = "# café ☃\n" + f()
C["good-version-core"] = f(v="1.2.3")
C["good-version-zero"] = f(v="0.0.0")
C["good-version-pre-numeric"] = f(v="1.2.3-0")
C["good-version-pre-hyphen"] = f(v="1.2.3-a-b.c")
C["good-version-pre-dots"] = f(v="1.0.0-x.7.z.92")
C["good-version-19-digits"] = f(v="1234567890123456789.0.0")
C["good-version-64-bytes"] = f(v="1.2.3-" + "a"*58)
C["good-registry-port"] = f(s="reg.example:5000/team/cortex-server@sha256:" + "c"*64)
C["good-image-hex-digits"] = f(d="x/y@sha256:0123456789abcdef"*1 + "0123456789abcdef"*3 + "")
C["good-value-300-bytes"] = f(s=("n"*(300-72)) + "@sha256:" + "a"*64)
base = f()
C["good-size-8192"] = base + "#" + "x"*(8192-len(base)-2) + "\n"
# ---- files both sides should REFUSE
C["bad-empty-file"] = ""
C["bad-only-comments"] = "# nothing\n\n"
C["bad-missing-version"] = f"CORTEX_SERVER_IMAGE={S}\nCORTEX_DB_IMAGE={D}\n"
C["bad-missing-server"] = f"CORTEX_VERSION=1.2.3\nCORTEX_DB_IMAGE={D}\n"
C["bad-missing-db"] = f"CORTEX_VERSION=1.2.3\nCORTEX_SERVER_IMAGE={S}\n"
C["bad-extra-key"] = f() + "EXTRA=1\n"
C["bad-secret-key"] = f() + "CORTEX_DB_PASSWORD=hunter2\n"
C["bad-dup-version"] = f() + "CORTEX_VERSION=1.2.3\n"
C["bad-dup-server"] = f() + f"CORTEX_SERVER_IMAGE={S}\n"
C["bad-dup-db"] = f() + f"CORTEX_DB_IMAGE={D}\n"
C["bad-lowercase-key"] = f().replace("CORTEX_VERSION", "cortex_version")
C["bad-key-trailing-space"] = f().replace("CORTEX_VERSION=", "CORTEX_VERSION =")
C["bad-key-leading-space"] = " " + f()
C["bad-export-prefix"] = "export " + f()
C["bad-value-leading-space"] = f().replace("=0.2.1-rc.2", "= 0.2.1-rc.2")
C["bad-value-trailing-space"] = f().replace("0.2.1-rc.2", "0.2.1-rc.2 ")
C["bad-value-tab"] = f().replace("0.2.1-rc.2", "0.2.1-rc.2\t")
C["bad-value-quoted"] = f().replace("0.2.1-rc.2", '"0.2.1-rc.2"')
C["bad-value-empty-version"] = f(v="")
C["bad-value-equals-inside"] = f().replace("0.2.1-rc.2", "0.2.1=rc")
C["bad-line-no-equals"] = f() + "CORTEX_VERSION\n"
C["bad-line-words"] = "this is not a pair\n" + f()
C["bad-crlf-file"] = f().replace("\n", "\r\n")
C["bad-cr-at-end-of-value"] = f().replace("0.2.1-rc.2\n", "0.2.1-rc.2\r\n")
C["bad-size-8193"] = base + "#" + "x"*(8193-len(base)-2) + "\n"
C["bad-value-301-bytes"] = f(s=("n"*(301-72)) + "@sha256:" + "a"*64)
for i, v in enumerate(["1", "1.0", "1.2.3.4", "01.2.3", "1.02.3", "1.2.03", "1.2.3-", "1.2.3-.a", "1.2.3-a.", "1.2.3-a..b", "1.2.3-01", "1.2.3+b", "1.2.3+build.7",
                       "1.2.3-rc.part", "1.2.3.part", ".1.2.3", "1.2.3.", "1..3", "a.b.c", "1.2.3-a_b", "1.2.3/x", "1.2.3-rc.01", "99999999999999999999.0.0", "1.2.3-" + "a"*59, "0.2.001", "v1.2.3"]):
    C[f"bad-version-{i:02d}"] = f(v=v)
C["bad-image-tag"] = f(s="localhost/cortex-server:latest")
C["bad-image-no-digest"] = f(s="localhost/cortex-server@sha256:")
C["bad-image-digest-63"] = f(d="x@sha256:" + "b"*63)
C["bad-image-digest-65"] = f(d="x@sha256:" + "b"*65)
C["bad-image-digest-upper"] = f(s="x@sha256:" + "A"*64)
C["bad-image-digest-nonhex"] = f(d="x@sha256:" + "b"*63 + "g")
C["bad-image-name-empty"] = f(s="@sha256:" + "a"*64)
C["bad-image-name-second-at"] = f(s="a@b@sha256:" + "a"*64)
C["bad-image-double-digest"] = f(s="x@sha256:" + "a"*64 + "@sha256:" + "b"*64)
C["bad-image-space"] = f(s="localhost/cortex server@sha256:" + "a"*64)
C["bad-image-sha512"] = f(s="x@sha512:" + "a"*128)
C["bad-image-plus"] = f(s="x+y@sha256:" + "a"*64)
# ---- added with the mutants (every character class and every whitespace kind the engine's trim() has in ASCII)
C["good-blank-vt-line"] = "\x0b\n" + f()
C["good-blank-ff-line"] = "\x0c\n" + f()
C["good-comment-after-ff"] = "\x0c# a comment after a form feed\n" + f()
C["good-version-all-digits"] = f(v="1234567890.9876543210.1234567890")
C["good-version-pre-numeric-digits"] = f(v="1.2.3-1234567890")
C["good-version-pre-alnum-leading-zero"] = f(v="1.2.3-0a")
C["good-version-every-letter"] = f(v="1.2.3-ABCDEFGHIJKLMNOPQRSTUVWXYZ-abcdefghijklmnopqrstuvwxyz")
C["good-image-underscore"] = f(s="my_registry/cortex_server@sha256:" + "a"*64)
C["good-image-every-character"] = f(s="ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789._:/-@sha256:" + "0123456789abcdef"*4)
C["bad-version-pre-zero-digits"] = f(v="1.2.3-0123456789")   # a numeric pre-release identifier with a leading zero, every digit in it
C["bad-image-equals"] = f(s="x=y@sha256:" + "a"*64)
C["bad-image-colon-digest-hex-upper-mixed"] = f(d="x@sha256:" + "0123456789abcdE" + "f"*49)
# ---- added in the 6d/7a fix series (hale P-1: bash `read` silently DROPS a NUL byte, the engine does not; vera and hale N-1/PRE6D-C1: the u64 boundary of a version core)
C["bad-nul-line"] = f() + "\x00\n"                                   # a line that is only a NUL: blank to a reader that drops it, BadLine to the engine
C["bad-nul-in-value"] = f(v="0.2.1\x00")                              # a NUL inside the value: the shell used to drop it and accept, the engine says BadValue
C["bad-version-core-u64-overflow"] = f(v="18446744073709551616.0.0")    # u64::MAX + 1: the engine's checked-u64 refuses it and so does install.sh (20 digits)
for k, v in C.items():
    with open(os.path.join(out, k + ".env"), "wb") as fh:
        fh.write(v.encode("utf-8"))
print(len(C), "files in", out)

# ---- known differences: what the engine accepts or refuses that install.sh does not (written down, held by IT-93b, never silently changed)
kd = os.path.join(out, "known-differences")
os.makedirs(kd, exist_ok=True)
K = {
  "nbsp-only-line": b"\xc2\xa0\n" + f().encode(),                       # engine: blank (Unicode white space); install.sh: not blank
  "nel-only-line": b"\xc2\x85\n" + f().encode(),                        # engine: blank (U+0085 is white space); install.sh: not blank
  "invalid-utf8-comment": b"# \xff\xfe not text\n" + f().encode(),     # engine: refuses (not UTF-8); install.sh: a comment is skipped unread
  "core-1e19-valid-in-engine": f(v="10000000000000000000.0.0").encode(),         # 10^19 fits u64: the engine accepts it; install.sh refuses every 20-digit core (valid_semver: at most 19 digits)
  "core-u64-max-valid-in-engine": f(v="18446744073709551615.0.0").encode(),      # u64::MAX: the same
  "nul-in-comment-valid-in-engine": b"# a comment with a NUL \x00 inside\n" + f().encode(),   # the engine skips a comment unread and accepts; install.sh refuses ANY file with a NUL byte (a whole-file test: hale's tested fix; bash read would drop the byte): stricter, fail closed
}
for k, v in K.items():
    with open(os.path.join(kd, k + ".env"), "wb") as fh:
        fh.write(v)
print(len(K), "known differences in", kd)

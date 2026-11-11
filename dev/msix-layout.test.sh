#!/usr/bin/env bash
# Tests for dev/msix-layout.sh on any system: the window's unzipped Windows archive goes in, the
# folder makeappx packs must come out, with a manifest Partner Center accepts. Run:
# dev/msix-layout.test.sh
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
failures=0
ok() { echo "ok   $1"; }
bad() { echo "FAIL $1"; failures=$((failures + 1)); }
check() {
    local name=$1 condition=$2
    if eval "$condition"; then ok "$name"; else bad "$name"; fi
}

# The window's Windows archive as dist makes it, unzipped: the files at the top.
mkdir -p "$work/in"
for f in senzor.exe LICENSE NOTICE README.md CHANGELOG.md; do echo "$f" > "$work/in/$f"; done

version=$(awk '/^\[workspace\.package\]/{i=1;next} /^\[/{i=0} i&&/^version *=/{gsub(/.*= *"|".*/,"");print;exit}' \
    "$here/../Cargo.toml")

for arch in x64 arm64; do
    out="$work/layout-$arch"
    if "$here/msix-layout.sh" "$work/in" "$arch" "$out" > "$work/log" 2>&1; then
        ok "$arch: it builds"
    else
        bad "$arch: it builds: $(cat "$work/log")"
        continue
    fi
    manifest="$out/AppxManifest.xml"
    attr() {
        python3 -I - "$manifest" "$1" "$2" <<'EOF'
import sys, xml.etree.ElementTree as ET
ns = {"m": "http://schemas.microsoft.com/appx/manifest/foundation/windows10",
      "uap": "http://schemas.microsoft.com/appx/manifest/uap/windows10"}
node = ET.parse(sys.argv[1]).getroot().find(sys.argv[2], ns)
print(node.get(sys.argv[3]) if node is not None else "")
EOF
    }
    text() {
        python3 -I - "$manifest" "$1" <<'EOF'
import sys, xml.etree.ElementTree as ET
ns = {"m": "http://schemas.microsoft.com/appx/manifest/foundation/windows10"}
print(" ".join(n.text or n.get("Language") for n in ET.parse(sys.argv[1]).getroot().findall(sys.argv[2], ns)))
EOF
    }
    check "$arch: the program, LICENSE and NOTICE" \
        "[[ -f '$out/senzor.exe' && -f '$out/LICENSE' && -f '$out/NOTICE' ]]"
    check "$arch: no README or CHANGELOG" "[[ ! -e '$out/README.md' && ! -e '$out/CHANGELOG.md' ]]"
    check "$arch: the manifest is well-formed XML" "python3 -I -c 'import sys,xml.dom.minidom as m; m.parse(sys.argv[1])' '$manifest'"
    check "$arch: no placeholder left" "! grep -q '@[A-Z]*@' '$manifest'"
    check "$arch: the Store's identity name" "[[ \$(attr m:Identity Name) == 43614JaroslavKaka.Senzor ]]"
    check "$arch: the Store's publisher" "[[ \$(attr m:Identity Publisher) == CN=AC5B9DA6-CE4E-453D-8D2C-1683417569BB ]]"
    check "$arch: the publisher's display name, with its háček" \
        "[[ \$(text m:Properties/m:PublisherDisplayName) == 'Jaroslav Kaňka' ]]"
    check "$arch: the version from Cargo.toml with a fourth part of 0" "[[ \$(attr m:Identity Version) == '$version.0' ]]"
    check "$arch: the architecture" "[[ \$(attr m:Identity ProcessorArchitecture) == $arch ]]"
    check "$arch: English and Czech" "[[ \$(text m:Resources/m:Resource) == 'en-us cs-cz' ]]"
    check "$arch: the executable exists" "[[ -f '$out/'\$(attr m:Applications/m:Application Executable) ]]"
    check "$arch: full trust" "grep -q 'Name=\"runFullTrust\"' '$manifest'"
    # Every logo the manifest names exists in each scale Windows asks for; the unqualified name
    # resolves through resources.pri.
    for logo in "m:Properties/m:Logo" "m:Applications/m:Application/uap:VisualElements@Square150x150Logo" \
        "m:Applications/m:Application/uap:VisualElements@Square44x44Logo"; do
        if [[ $logo == *@* ]]; then path=$(attr "${logo%@*}" "${logo#*@}"); else path=$(text "$logo"); fi
        base=${path//\\//}
        base=${base%.png}
        for scale in 100 200 400; do
            check "$arch: $base.scale-$scale.png" "[[ -f '$out/$base.scale-$scale.png' ]]"
        done
    done
    for size in 16 24 32 48 256; do
        check "$arch: taskbar icon $size, plated and unplated" \
            "[[ -f '$out/Assets/Square44x44Logo.targetsize-$size.png' && -f '$out/Assets/Square44x44Logo.targetsize-${size}_altform-unplated.png' ]]"
    done
    check "$arch: priconfig.xml stays out of the package" "[[ ! -e '$out/priconfig.xml' ]]"
done

# A pre-release version has no Store form (four numbers).
if SENZOR_VERSION=1.1.0-rc.1 "$here/msix-layout.sh" "$work/in" x64 "$work/pre/out" > "$work/log" 2>&1; then
    bad "a pre-release version is refused"
else
    ok "a pre-release version is refused"
fi
if "$here/msix-layout.sh" "$work/in" x86 "$work/x86" > "$work/log" 2>&1; then
    bad "an unknown architecture is refused"
else
    ok "an unknown architecture is refused"
fi

if [[ $failures -gt 0 ]]; then
    echo "$failures failed"
    exit 1
fi
echo "all passed"

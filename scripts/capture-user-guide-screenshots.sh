#!/bin/sh
set -eu

script_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_directory=$(CDPATH= cd -- "$script_directory/.." && pwd)
output_directory=${1:-"$repo_directory/docs/images/user-guide"}

mkdir -p "$output_directory"
output_directory=$(CDPATH= cd -- "$output_directory" && pwd)

case "$(uname -s)" in
    Darwin|Linux) ;;
    *)
        echo "The deterministic UI screenshot preview is supported on macOS and Linux." >&2
        exit 1
        ;;
esac

preview_bundle_root=""
preview_bundle=""
if [ "$(uname -s)" = "Darwin" ]; then
    cargo build -p stageswap --bin StageSwap
    macos_temp_root=/tmp
    if [ -d /private/tmp ]; then
        macos_temp_root=/private/tmp
    fi
    preview_bundle_root=$(mktemp -d "$macos_temp_root/stageswap-doc-capture.XXXXXX")
    preview_bundle="$preview_bundle_root/StageSwapPreview.app"
    mkdir -p "$preview_bundle/Contents/MacOS"
    cp "$repo_directory/target/debug/StageSwap" "$preview_bundle/Contents/MacOS/StageSwap"
    cp "$script_directory/stageswap-preview-info.plist" "$preview_bundle/Contents/Info.plist"
fi

cleanup_preview_bundle() {
    if [ -n "$preview_bundle_root" ] && [ -d "$preview_bundle_root" ]; then
        rm -rf "$preview_bundle_root"
    fi
}
trap cleanup_preview_bundle EXIT

verify_png() {
    png_path=$1
    python3 - "$png_path" <<'PY'
import struct
import sys

path = sys.argv[1]
with open(path, "rb") as image:
    if image.read(8) != b"\x89PNG\r\n\x1a\n":
        raise SystemExit(f"{path}: not a PNG")
    length = struct.unpack(">I", image.read(4))[0]
    if length < 8 or image.read(4) != b"IHDR":
        raise SystemExit(f"{path}: missing PNG IHDR")
    width, height = struct.unpack(">II", image.read(8))
    if (width, height) != (1280, 720):
        raise SystemExit(f"{path}: expected 1280x720, got {width}x{height}")
PY
}

capture() {
    target=$1
    filename=$2
    shift 2
    output_path="$output_directory/$filename"
    echo "Capturing $target -> $output_path"
    rm -f "$output_path"
    if [ -n "$preview_bundle" ]; then
        open -n "$preview_bundle" --args \
            --ui-preview "$target" \
            --ui-language en-US \
            --ui-screenshot "$output_path" \
            "$@"
        attempts=0
        while [ ! -s "$output_path" ]; do
            attempts=$((attempts + 1))
            if [ "$attempts" -ge 90 ]; then
                echo "Timed out waiting for $output_path" >&2
                exit 1
            fi
            sleep 1
        done
    else
        cargo run -p stageswap --bin StageSwap -- \
            --ui-preview "$target" \
            --ui-language en-US \
            --ui-screenshot "$output_path" \
            "$@"
    fi
    verify_png "$output_path"
}

capture dashboard dashboard.png
capture general settings-general.png
capture webcam settings-webcam.png
capture screen settings-secondary-screen.png
capture matching settings-reference-image.png
capture diagnostics settings-diagnostics.png

capture setup-1 setup-1-jw-library-to-zoom.png
capture setup-2 setup-2-webcam.png
capture setup-3 setup-3-secondary-screen.png
capture setup-4 setup-4-reference-empty.png --ui-setup-reference-state empty
capture setup-4 setup-4-reference-review.png --ui-setup-reference-state review
capture setup-4 setup-4-reference-confirmed.png --ui-setup-reference-state captured
capture setup-4 setup-4-reference-missing-screen.png --ui-setup-reference-state missing-screen
capture setup-5 setup-5-ready.png

capture dialog-reference-capture dialog-reference-capture.png
capture dialog-exit dialog-exit.png
capture dialog-clear-logs dialog-clear-logs.png
capture dialog-admin dialog-admin-saved.png
capture dialog-replace-baseline dialog-replace-saved-configuration.png
capture dialog-load-admin-config dialog-load-saved-configuration.png
capture dialog-remove-baseline dialog-delete-saved-configuration.png

echo "All user-guide screenshots are 1280x720 PNGs in $output_directory"

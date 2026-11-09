#!/usr/bin/env bash
# Fetch the official Windows 11 ARM64 ISO from Microsoft's software-download page.
# Request sequence, User-Agent and endpoints follow dockur/windows src/mido.sh
# (downloadWindowsLink, productId 3324, DownloadType 2).
#
# Usage: fetch-iso.sh [--link-only] [language]   language: "English" (default) | "English International"
# Writes into $WIN_VM_DIR (default ~/.cache/nudge-win-vm), never into the repo:
# download-link.txt, sku.json, links.json and the ISO, win11-arm64.iso (~5.5 GB).
set -euo pipefail

DIR="${WIN_VM_DIR:-$HOME/.cache/nudge-win-vm}"
mkdir -p "$DIR"
ISO="$DIR/win11-arm64.iso"
LINK_ONLY=0
if [ "${1:-}" = "--link-only" ]; then LINK_ONLY=1; shift; fi
LANGUAGE="${1:-English}"

PRODUCT_ID="${PRODUCT_ID:-}"  # product edition id; read from the page when unset
DOWNLOAD_TYPE="2"          # ARM64 entry in ProductDownloadOptions
PAGE="https://www.microsoft.com/en-us/software-download/windows11arm64"
PROFILE="606624d44113"
ORG_ID="y6jn8c31"
INSTANCE="560dc9f3-1aa5-4a2f-b63c-9e18f8d0e175"

# Synthetic Firefox UA, advanced every 2 weeks (qemus/qemu src/download.sh getAgent)
ver=$((152 + ($(date +%s) - 1781568000) / 1209600))
AGENT="Mozilla/5.0 (X11; Linux x86_64; rv:${ver}.0) Gecko/20100101 Firefox/${ver}.0"

req() {
  LC_ALL=C curl --silent --show-error --max-time 30 --user-agent "$AGENT" \
    --fail --proto =https --tlsv1.2 --http1.1 --header "Accept:" "$@"
}

if [ -s "$DIR/download-link.txt" ] && [ "$LINK_ONLY" = 0 ]; then
  echo "Reusing link from download-link.txt (valid 24h from issue)"
  LINK="$(cat "$DIR/download-link.txt")"
else
  SESSION="$(uuidgen | tr 'A-Z' 'a-z')"
  echo "UA: $AGENT"
  echo "Session: $SESSION"

  # 0. current product edition id from the page's <option> (mido.sh fallback path);
  #    3324 is mido.sh's hardcoded id, which still serves 25H2
  if [ -z "$PRODUCT_ID" ]; then
    PRODUCT_ID="$(req --max-filesize 1M -- "$PAGE" | tr '\r\n' '  ' |
      LC_ALL=C grep -Eio "<option[^>]*value=[\"'][0-9]+[\"'][^>]*>[[:space:]]*Windows[^<]*" |
      sed -nE "s/.*value=[\"']([0-9]+)[\"'].*/\\1/p" | head -1)" || PRODUCT_ID=""
    PRODUCT_ID="${PRODUCT_ID:-3324}"
  fi
  echo "Product edition id: $PRODUCT_ID"

  # 1. register session with the anti-abuse endpoint
  req --output /dev/null --max-filesize 100K -- \
    "https://vlscppe.microsoft.com/tags?org_id=$ORG_ID&session_id=$SESSION"

  # 2. ov-df challenge: fetch token + rticks, reply with current ms timestamp
  ov="$(req --max-filesize 1M -- \
    "https://ov-df.microsoft.com/mdt.js?instanceId=$INSTANCE&PageId=si&session_id=$SESSION")"
  [[ $ov =~ [\?\&]w=([A-Fa-f0-9]+) ]] && token="${BASH_REMATCH[1]}" || token=""
  [[ $ov =~ rticks=\"\+?([0-9]+) ]] && ticks="${BASH_REMATCH[1]}" || ticks=""
  if [ -z "$token" ] || [ -z "$ticks" ]; then echo "ov-df data not found" >&2; exit 1; fi
  sleep 0.2
  ms="$(perl -MTime::HiRes=time -e 'printf "%d", time()*1000')"
  req --output /dev/null --max-filesize 100K -- \
    "https://ov-df.microsoft.com/?session_id=$SESSION&CustomerId=$INSTANCE&PageId=si&w=$token&mdt=$ms&rticks=$ticks"

  # 3. SKU id for the language
  req --referer "$PAGE" --max-filesize 100K -o "$DIR/sku.json" -- \
    "https://www.microsoft.com/software-download-connector/api/getskuinformationbyproductedition?profile=$PROFILE&ProductEditionId=$PRODUCT_ID&SKU=undefined&friendlyFileName=undefined&Locale=en-US&sessionID=$SESSION"
  SKU="$(jq -r --arg L "$LANGUAGE" 'first(.Skus[]? | select(.Language == $L) | .Id) // empty' "$DIR/sku.json")"
  if [ -z "$SKU" ]; then echo "No SKU for '$LANGUAGE':" >&2; cat "$DIR/sku.json" >&2; exit 1; fi
  echo "SKU ($LANGUAGE): $SKU  $(jq -r --arg L "$LANGUAGE" 'first(.Skus[]? | select(.Language == $L) | .ProductDisplayName)' "$DIR/sku.json")"

  # 4. download link (Microsoft applies IP blocking here; do not retry in a loop)
  req --referer "$PAGE" --max-filesize 100K -o "$DIR/links.json" -- \
    "https://www.microsoft.com/software-download-connector/api/GetProductDownloadLinksBySku?profile=$PROFILE&ProductEditionId=undefined&SKU=$SKU&friendlyFileName=undefined&Locale=en-US&sessionID=$SESSION"
  if grep -Fq "Sentinel marked this request as rejected." "$DIR/links.json" ||
     grep -Fq "We are unable to complete your request at this time." "$DIR/links.json"; then
    echo "Microsoft blocked the request (IP ban, ~24h). Do not retry now." >&2; cat "$DIR/links.json" >&2; exit 2
  fi
  LINK="$(jq -r --argjson T "$DOWNLOAD_TYPE" 'first(.ProductDownloadOptions[]? | select(.DownloadType == $T) | .Uri) // empty' "$DIR/links.json")"
  if [ -z "$LINK" ]; then echo "No download link:" >&2; cat "$DIR/links.json" >&2; exit 1; fi
  printf '%s\n' "$LINK" > "$DIR/download-link.txt"
  echo "Link saved to download-link.txt"
fi

[ "$LINK_ONLY" = 1 ] && exit 0

# 5. resumable download
curl -L -C - --fail --retry 5 --retry-delay 10 --user-agent "$AGENT" -o "$ISO" "$LINK"
echo "Done: $ISO"

#!/usr/bin/env bash
# Print the IPv4 of a UTM VM: the guest agent first (utmctl), then the vmnet DHCP
# lease for the VM's MAC address, which works before the agent is up.
# usage: vm-ip.sh [vm-name]     (default: $WIN_VM, else nudge-win11)
VM=${1:-${WIN_VM:-nudge-win11}}
ip=$(utmctl ip-address "$VM" 2>/dev/null | grep -E '^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$' | grep -v '^169\.254' | head -1)
if [ -z "$ip" ]; then
  mac=$(plutil -extract Network.0.MacAddress raw "$HOME/Library/Containers/com.utmapp.UTM/Data/Documents/$VM.utm/config.plist" 2>/dev/null)
  [ -n "$mac" ] || exit 1
  # dhcpd_leases drops leading zeros in each octet and lowercases.
  norm=$(echo "$mac" | tr 'A-F' 'a-f' | awk -F: '{for(i=1;i<=NF;i++){sub(/^0/,"",$i)}; OFS=":"; $1=$1; print}')
  ip=$(awk -v m="1,$norm" '/ip_address=/{split($0,a,"=");ip=a[2]} /hw_address=/{split($0,b,"="); if(b[2]==m) last=ip} END{print last}' /var/db/dhcpd_leases)
fi
[ -n "$ip" ] && echo "$ip"

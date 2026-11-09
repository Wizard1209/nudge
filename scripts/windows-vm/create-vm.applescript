-- Creates the nudge-win11 QEMU VM (aarch64, virtualized). TPM/secure-boot keys and
-- RTC-local-time are not exposed to AppleScript; create-vm.sh patches config.plist after this.
on run argv
	set winIso to POSIX file (item 1 of argv)
	set setupIso to POSIX file (item 2 of argv)
	tell application "UTM"
		set vm to make new virtual machine with properties {backend:qemu, configuration:{name:"nudge-win11", architecture:"aarch64", memory:8192, cpu cores:4, hypervisor:true, uefi:true, drives:{{removable:true, source:winIso}, {guest size:65536, interface:NVMe}, {removable:true, source:setupIso}}, displays:{{hardware:"virtio-ramfb-gl"}}, network interfaces:{{mode:shared}}}}
		return id of vm
	end tell
end run

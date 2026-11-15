-- After the install: drop both installer CD drives (no "Press any key to boot from CD" on later
-- boots) and turn off dynamic resolution so the guest keeps the 1920x1080 it sets itself.
-- The VM must be stopped.
tell application "UTM"
	set vm to virtual machine named "nudge-win11"
	set config to configuration of vm
	set keep to {}
	repeat with d in drives of config
		if removable of d is false then set end of keep to (contents of d)
	end repeat
	set drives of config to keep
	set disp to item 1 of displays of config
	set dynamic resolution of disp to false
	set item 1 of displays of config to disp
	update configuration of vm with config
	return "drives kept: " & (count of keep)
end tell

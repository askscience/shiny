#!/bin/sh
# Wait for the AMD dGPU render node before Ollama starts.
#
# Why: on this MacBook Pro (T2), amdgpu finishes initializing ~10s after the
# kernel starts, which is LATER than network-online.target (the only dependency
# of ollama.service). If Ollama probes Vulkan too early it only finds the Intel
# iGPU + CPU and then runs CPU-only until manually restarted.
#
# Card/render numbering is NOT stable across boots (AMD has been both card0 and
# card1), so match on the PCI vendor id 0x1002 instead of a fixed device path.
#
# Always exits 0 so a slow GPU can never block boot.

i=0
while [ "$i" -lt 60 ]; do
	for v in /sys/class/drm/card*/device/vendor; do
		[ -r "$v" ] || continue
		[ "$(cat "$v" 2>/dev/null)" = "0x1002" ] || continue
		c=${v%/device/vendor}
		for r in "$c"/device/drm/renderD*; do
			[ -e "$r" ] || continue
			sleep 3   # let the driver settle before Vulkan enumeration
			echo "AMD GPU render node ready: $(basename "$r")"
			exit 0
		done
	done
	sleep 1
	i=$((i + 1))
done

echo "warning: timed out waiting for AMD GPU; continuing anyway" >&2
exit 0

#!/bin/sh
# What happens after a Shiny seat unit stops. This is the ExecStopPost target
# of `shiny-greeter.service` and `shiny-kiosk@.service` — and, through the
# `OnFailure=` fallback units, the target for a unit that died so hard it never
# reached a normal stop (start limit hit). Run as root (the `+` prefix in the
# unit) because it may start other units.
#
#   shiny-seat-stop.sh greeter   after the login screen stops:
#       handover marker present → say nothing: a kiosk is taking the seat;
#       clean stop / crash-loop → put getty@tty1 back so the console works;
#       machine going down      → do nothing.
#
#   shiny-seat-stop.sh kiosk     after a user session stops:
#       clear the handover marker, then bring the login screen back — unless
#       the machine is going down, or this stop is a crash that systemd's
#       Restart=on-failure is about to retry (SERVICE_RESULT=exit-code).
#
# A second argument overrides `$SERVICE_RESULT` for the `OnFailure=` path,
# which runs outside the failed unit's own stop context.
#
# Installed to /usr/local/bin/shiny-seat-stop.sh by
# scripts/install-kiosk-greeter.sh.
set -u

role=${1:-}
marker=/run/shiny/handover
result=${2:-${SERVICE_RESULT:-unknown}}

# During shutdown/reboot nothing should be (re)started. `is-system-running`
# says "running" or "degraded" on a live machine; anything else (starting,
# stopping, maintenance) means this script must stay out of the way.
state=$(/usr/bin/systemctl is-system-running 2>/dev/null || true)
case "$state" in
    running|degraded) ;;
    *) exit 0 ;;
esac

case "$role" in
    greeter)
        # A stop with the marker present is a handover to a kiosk session:
        # tty1 belongs to the kiosk now, so no console here.
        [ -e "$marker" ] && exit 0
        case "$result" in
            success|start-limit-hit|failed)
                /usr/bin/systemctl --no-block start getty@tty1.service
                ;;
        esac
        ;;
    kiosk)
        rm -f "$marker"
        case "$result" in
            success|start-limit-hit|failed)
                /usr/bin/systemctl --no-block start shiny-greeter.service
                ;;
        esac
        ;;
esac
exit 0

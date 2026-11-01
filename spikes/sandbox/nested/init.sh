#!/bin/bash
# The sandbox's first process. Services the session bus activates inherit its
# stdio, and one stray line on stdout would corrupt the MCP stream, so the
# stream moves to fds 8 and 9, which only the driver reads and writes.
exec 8<&0 9>&1 </dev/null 1>&2
exec /usr/bin/dbus-run-session -- /keek/nested/session.sh

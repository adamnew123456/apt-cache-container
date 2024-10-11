#!/bin/sh
set -x
/approx_host.bin syslog &
syslog_pid=$!

/approx_host.bin gc "${GC_INTERVAL:-06:00:00}" "${GC_MAXAGE:-30:00:00:00}" &
gc_pid=$!

/approx_host.bin inetd "${CACHE_PORT:-80}" /usr/sbin/approx

kill -9 $syslog_pid
kill -9 $gc_pid

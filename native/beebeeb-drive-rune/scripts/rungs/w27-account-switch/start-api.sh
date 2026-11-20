#!/usr/bin/env bash
# start my own test API on 47851 (test stack on localhost, never production); the signup cap is off (debug build flag)
ST=${W27_STACK:?set W27_STACK to the test stack dir}; BIN=${W27_API_BIN:?set W27_API_BIN to a debug beebeeb-api build}
cd $ST/run || exit 1
env -i PATH="$PATH" HOME="$HOME" DATABASE_URL=${W27_DATABASE_URL:?set W27_DATABASE_URL} BB_PORT=47851 BLOB_STORE=local BLOB_STORE_PATH=$ST/blobs OPAQUE_SERVER_SETUP="$(tail -1 $ST/opaque)" SMTP_HOST=127.0.0.1 SMTP_PORT=47825 SMTP_TLS_MODE=none BB_ALLOW_REAL_EMAIL=1 BB_SIGNUP_IN_APPS=1 BB_SIGNUP_EMAIL_CODE=1 BB_RATE_LIMIT_DISABLED=1 EMAIL_FROM=test@example.org APP_URL=http://127.0.0.1:3051 setsid nohup $BIN >> $ST/api4.log 2>&1 < /dev/null &

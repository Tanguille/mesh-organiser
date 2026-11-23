cmd /c build-web-svelte.bat
cd web

rem Set LOCAL_ACCOUNT_PASSWORD privately in the calling environment for local admin access.
set APP_CONFIG_PATH=Z:/config.json
::set RUST_BACKTRACE=full
mklink /D www ..\build
cargo run
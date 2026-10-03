cmd /c build-web-svelte.bat
cd web

set LOCAL_ACCOUNT_PASSWORD=abc
set APP_CONFIG_PATH=Z:/config.json
::set RUST_BACKTRACE=full
mklink /D www ..\build
cargo run
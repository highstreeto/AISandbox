$env:SANDBOX_DIR = "$PSScriptRoot/.sandbox"
cargo run --quiet --locked --manifest-path "$PSScriptRoot/Cargo.toml" --bin sandbox -- $args

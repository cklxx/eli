# Build release binary
build:
    cargo build --release

# Install globally
install:
    cargo install --path crates/eli

# Run tests
test:
    cargo test

# Run with arguments
run *ARGS:
    cargo run -p eli -- {{ARGS}}

# Clean
clean:
    cargo clean

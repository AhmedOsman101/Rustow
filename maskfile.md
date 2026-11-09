# rustow — task runner

All commands assume repo root as cwd. Use `$MASK` in scripts that chain other mask commands.

## build

> Build the debug binary

~~~sh
cargo build
~~~

## test

> Run all tests

~~~sh
cargo test
~~~

## lint

> Check formatting and lints

~~~sh
cargo fmt --check && cargo clippy --all-targets -- -D warnings
~~~

## format [args]

> Format the project

~~~sh
eval
rustfmt --config-path rustfmt.toml
~~~

## clean

> Clean build artifacts

~~~sh
cargo clean
~~~

## install

> Install rustow into PATH (~/.cargo/bin)

~~~sh
cargo install --path .
~~~

## schema

> Regenerate schema.json at the repo root from the Config struct

~~~sh
cargo run --quiet --example generate_schema
~~~

### check

> Verify schema.json is up to date with the Config struct

~~~sh
cargo run --quiet --example generate_schema -- --check
~~~

## e2e

> Run the end-to-end scenario in a Void Linux podman container

~~~sh
podman build -t rustow-e2e -f Dockerfile .
podman run --rm -v "$PWD:/app" -w /app rustow-e2e bash tests/e2e.sh
~~~

## check

> Run lint + tests

~~~sh
$MASK lint && $MASK test
~~~

name := "chzzk"
target := "aarch64-unknown-linux-musl"
host := "oracle"
remote := "~/server"

build:
    cargo zigbuild --release --target {{target}}

publish: build
    ssh {{host}} "cat > {{remote}}/{{name}}/{{name}}.tmp && mv {{remote}}/{{name}}/{{name}}.tmp {{remote}}/{{name}}/{{name}} && cd {{remote}} && docker compose up -d --build {{name}}" < target/{{target}}/release/{{name}}

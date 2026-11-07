# The sandbox image: a hub (hoi-node) and the pentest harness (hoi-sim) in one small container.
# Build from the core/ directory so the Cargo workspace is the context:
#     docker build -f sandbox/Dockerfile -t hoi-sandbox core
# The hoi-sandbox script does this for you.
FROM rust:1.97-slim-bookworm@sha256:2775a09d208ff0d7c1f50490c45b62db929e87ba1dcbc3f2132ac71a704bcdd3 AS build
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p hoi-node -p hoi-sim

FROM debian:bookworm-slim@sha256:734a72b3d6142a71993b4bc580ca01d8ca687f77a2c422d96a961d3376b179bc
# A non-root user; the sandbox containers also drop all capabilities and run read-only.
RUN useradd --system --home-dir /data --shell /usr/sbin/nologin hoi && mkdir -p /data && chown hoi /data
COPY --from=build /src/target/release/hoi-node /usr/local/bin/hoi-node
COPY --from=build /src/target/release/hoi-sim  /usr/local/bin/hoi-sim
USER hoi
WORKDIR /data
VOLUME /data
EXPOSE 7842
# No default entrypoint command: the hub runs `hoi-node …`, agents run `hoi-sim agent …`.
ENTRYPOINT []

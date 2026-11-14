# hoi-node in a container. Build from the repository root:
#   docker build -f core/crates/hoi-node/Dockerfile -t hoi-node .
# By default the paired-device listener stays on the container's own loopback (127.0.0.1:7842),
# which nothing outside the container reaches; only the peer transports listen on its interfaces.
# A loopback address inside a container cannot be published with -p, so to pair your devices
# from this machine, listen on the container's interfaces and publish the port on the host's
# loopback only:
#   docker run -d --name hoi-node -v hoi-node:/data -p 127.0.0.1:7842:7842 hoi-node --listen 0.0.0.0:7842 --name "lena's pi"
#   docker exec hoi-node cat /data/pairing-link   # the hoi://pair link: a credential, kept out of the logs
# Never publish 7842 on every host interface (-p 7842:7842, or -P) and never combine
# --network host with --listen 0.0.0.0:7842: Docker's published ports bypass most host firewalls.
# A public node (--public) has no pairing listener at all; deploy/server/compose.yml runs one;
# deploy/home/compose.yml runs a home node that is a public relay too (--public-relay).
# The build stage is the same Debian release as the image (bookworm): a binary linked against a
# newer C library than the image has would not start.
FROM rust:1.99-slim-bookworm@sha256:f2c004e3d2d8b547e8cf7ffbc29205385f4e71d23e53e4e383065db5e9f822cd AS build
WORKDIR /src/core
COPY core/ ./
COPY vendor/ /src/vendor/
# hoi-core embeds the repository's release manifest at compile time.
COPY release.json /src/release.json
RUN cargo build --release --locked -p hoi-node

# Just the binary, for a release built on your own computer (node scripts/release-local.mjs
# --docker): docker buildx build --platform linux/arm64 --target binary --output type=local,dest=out .
FROM scratch AS binary
COPY --from=build /src/core/target/release/hoi-node /hoi-node

FROM debian:bookworm-slim@sha256:734a72b3d6142a71993b4bc580ca01d8ca687f77a2c422d96a961d3376b179bc
RUN useradd --system --home-dir /data --shell /usr/sbin/nologin hoi && mkdir -p /data && chown hoi /data
COPY --from=build /src/core/target/release/hoi-node /usr/local/bin/hoi-node
USER hoi
VOLUME /data
# Only the peer transports are exposed; the paired-device port is published by hand, on loopback.
EXPOSE 7843/udp
EXPOSE 7844/udp
# the DHT among public nodes and its answer port
EXPOSE 7846/udp
EXPOSE 7847/udp
ENTRYPOINT ["hoi-node", "--data", "/data", "--peer-bind", "0.0.0.0:7843"]

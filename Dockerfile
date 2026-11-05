# The Firelight image (docs/HANDOVER.md §11): the prebuilt release binary on a
# minimal, non-root base. Nothing is compiled here, so a multi-platform build
# needs no emulation: CI's build-linux-* jobs produce dist/firelight-linux-<arch>.
#
# Base: distroless cc (glibc, CA certificates, no shell), pinned by digest
# (D5; revisited if the git CLI is needed, D1). Dependabot proposes updates.
FROM gcr.io/distroless/cc-debian12:nonroot@sha256:9dac0a79194e45a7da0158a9c6da57b217585af0786db3845d1f0ec1a0dd182f

ARG TARGETARCH
COPY --chmod=0755 dist/firelight-linux-${TARGETARCH} /usr/local/bin/firelight
COPY --chown=65532:65532 docker/data/ /var/lib/firelight/

# Data (database cache, clones, models) lives on a volume; the root filesystem
# can be read-only.
VOLUME ["/var/lib/firelight"]
ENV FIRELIGHT_SERVER_LISTEN=0.0.0.0:8080 \
    FIRELIGHT_DATA_DIR=/var/lib/firelight
EXPOSE 8080
USER 65532:65532
ENTRYPOINT ["/usr/local/bin/firelight"]
CMD ["serve"]

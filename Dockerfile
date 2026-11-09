# The Firelight image (docs/HANDOVER.md §11): the prebuilt release binary on a
# minimal, non-root base. Nothing is compiled here, so a multi-platform build
# needs no emulation: CI's build-linux-* jobs produce dist/firelight-linux-<arch>.
#
# Base (D5): Chainguard's git image (glibc, git 2.56, CA certificates, user
# 65532), pinned by digest. Git sources need the git CLI (D1: gix has no
# blobless clone or sparse checkout yet). Dependabot proposes updates.
FROM cgr.dev/chainguard/git:latest-glibc@sha256:f3b9ade5b3e78c5ea04cd77baa431ec3a5c060da5144a8357cc0ba47326dcd93

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

FROM alpine AS alpine
ARG DUMB_INIT_VERSION=1.2.5
RUN ARCH=$(uname -m) && \
    if [ "$ARCH" = "x86_64" ]; then DUMB_ARCH=x86_64; \
    elif [ "$ARCH" = "aarch64" ]; then DUMB_ARCH=aarch64; \
    else echo "unsupported arch: $ARCH" && exit 1; fi && \
    wget -O /usr/local/bin/dumb-init \
        https://github.com/Yelp/dumb-init/releases/download/v${DUMB_INIT_VERSION}/dumb-init_${DUMB_INIT_VERSION}_${DUMB_ARCH} && \
    chmod +x /usr/local/bin/dumb-init

FROM cgr.dev/chainguard/glibc-dynamic

WORKDIR /app

COPY --from=alpine /usr/local/bin/dumb-init /usr/local/bin/dumb-init

COPY --chown=nonroot:nonroot chzzk ./

USER nonroot
EXPOSE 3000

ENTRYPOINT ["/usr/local/bin/dumb-init", "--"]
CMD ["/app/chzzk"]

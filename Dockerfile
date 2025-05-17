FROM alpine AS alpine
RUN wget -O /usr/local/bin/dumb-init https://github.com/Yelp/dumb-init/releases/download/v1.2.5/dumb-init_1.2.5_x86_64
RUN chmod +x /usr/local/bin/dumb-init

FROM cgr.dev/chainguard/glibc-dynamic

WORKDIR /app

COPY --from=alpine /usr/local/bin/dumb-init /usr/local/bin/dumb-init

COPY --chown=nonroot:nonroot hd-bypass ./

USER nonroot
EXPOSE 3000

ENTRYPOINT ["/usr/local/bin/dumb-init", "--"]
CMD ["/app/hd-bypass"]

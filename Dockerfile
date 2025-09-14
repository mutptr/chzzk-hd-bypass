FROM cgr.dev/chainguard/glibc-dynamic

WORKDIR /app

COPY --chown=nonroot:nonroot chzzk ./

USER nonroot
EXPOSE 3000

ENTRYPOINT ["/app/chzzk"]

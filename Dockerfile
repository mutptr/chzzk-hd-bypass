FROM cgr.dev/chainguard/glibc-dynamic

COPY chzzk /app/chzzk

WORKDIR /app

USER nonroot
EXPOSE 3000

ENTRYPOINT ["/app/chzzk"]

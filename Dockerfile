FROM cgr.dev/chainguard/glibc-dynamic
COPY --chmod=0555 chzzk /app/chzzk
WORKDIR /app
EXPOSE 3000
ENTRYPOINT ["/app/chzzk"]

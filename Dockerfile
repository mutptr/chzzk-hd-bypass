FROM cgr.dev/chainguard/glibc-dynamic

WORKDIR /home/nonroot

COPY --chown=nonroot:nonroot chzzk ./

USER nonroot
EXPOSE 3000

ENTRYPOINT ["/home/nonroot/chzzk"]

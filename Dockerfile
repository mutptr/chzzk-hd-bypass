FROM cgr.dev/chainguard/glibc-dynamic

WORKDIR /home/nonroot

COPY chzzk ./

USER nonroot
EXPOSE 3000

ENTRYPOINT ["/home/nonroot/chzzk"]

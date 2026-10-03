FROM gcr.io/distroless/static-debian13:nonroot
COPY --chmod=0555 chzzk /app/chzzk
WORKDIR /app
EXPOSE 3000
ENTRYPOINT ["/app/chzzk"]

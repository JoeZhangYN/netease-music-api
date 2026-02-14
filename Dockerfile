FROM rust:1.83-alpine AS builder
RUN apk add --no-cache musl-dev pkgconfig
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
COPY src/ src/
COPY templates/ templates/
RUN cargo build --release

FROM alpine:3.21
RUN apk add --no-cache ca-certificates tzdata
ENV TZ=Asia/Shanghai
WORKDIR /app
COPY --from=builder /build/target/release/netease-music-api .
EXPOSE 5000
CMD ["./netease-music-api"]

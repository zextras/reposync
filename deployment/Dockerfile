FROM rust:1.93-alpine AS builder
LABEL maintainer = "Golden Beatles <goldenbeatles@zextras.com>"
LABEL description = "Reposync image"

# Install build dependencies
RUN apk add --no-cache \
    musl-dev \
    openssl-dev \
    openssl-libs-static \
    zlib-static \
    pkgconf \
    cmake \
    make \
    perl \
    gcc

# Set environment variables for static OpenSSL linking
ENV OPENSSL_STATIC=1
ENV OPENSSL_LIB_DIR=/usr/lib
ENV OPENSSL_INCLUDE_DIR=/usr/include

WORKDIR /project

# Copy dependency files first for better caching
COPY Cargo.toml ./
COPY Cargo.lock* ./

# Create dummy main.rs to enable dependency compilation
RUN mkdir src && \
    echo "fn main() {}" > src/main.rs && \
    cargo build --release && \
    rm -rf src target/release/deps/reposync*

# Copy source code
COPY src/ src/
COPY samples/ samples/

# Build the actual binary
RUN cargo build --release

# Final stage
FROM alpine:latest
COPY --from=builder /project/target/release/reposync /usr/bin/reposync

# Install runtime dependencies
RUN apk add --no-cache \
    ca-certificates \
    libgcc \
    openssl

ENTRYPOINT ["reposync"]
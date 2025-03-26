FROM rust:1.85-slim-bullseye

# Install system dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    git \
    && rm -rf /var/lib/apt/lists/*

# Install additional tools
RUN cargo install cargo-watch cargo-edit cargo-expand

# Set working directory
WORKDIR /workspace

# Create a non-root user
RUN useradd -m -u 1000 vscode

# Fix Cargo registry permissions
RUN chown -R vscode:vscode /usr/local/cargo

USER vscode

# Set environment variables
ENV RUST_BACKTRACE=1
ENV RUST_LOG=debug

# Default command
CMD ["/bin/bash"] 
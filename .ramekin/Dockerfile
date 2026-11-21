FROM ramekin-agent
RUN apt-get update && apt-get install -y --no-install-recommends lld \
    && rm -rf /var/lib/apt/lists/*
# BFD ld OOMs linking jj-lib's dependency tree in 1GB; a bigger container may make lld unnecessary.
ENV RUSTFLAGS="-C link-arg=-fuse-ld=lld"
RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable --profile minimal
ENV PATH="/root/.cargo/bin:${PATH}"

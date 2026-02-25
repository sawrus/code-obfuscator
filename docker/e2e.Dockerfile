FROM rust:1.89-bookworm

RUN apt-get update && apt-get install -y --no-install-recommends \
    bash \
    ca-certificates \
    curl \
    default-jdk \
    g++ \
    gcc \
    golang-go \
    nodejs \
    npm \
    sqlite3 \
    && rm -rf /var/lib/apt/lists/*

# TypeScript tooling for runtime checks.
RUN npm install -g typescript ts-node

# Install .NET SDK + Roslyn csc so C# checks can run in e2e.
RUN curl -fsSL https://dot.net/v1/dotnet-install.sh -o /tmp/dotnet-install.sh \
    && bash /tmp/dotnet-install.sh --channel 8.0 --install-dir /usr/share/dotnet \
    && ln -sf /usr/share/dotnet/dotnet /usr/local/bin/dotnet \
    && dotnet --info

ENV PATH="/usr/share/dotnet:${PATH}"
WORKDIR /work

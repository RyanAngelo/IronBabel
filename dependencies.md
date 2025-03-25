# Dependencies

This project uses the following dependencies:

## Core Dependencies

- [tokio](https://github.com/tokio-rs/tokio) (MIT) - Async runtime for handling concurrent operations and async I/O
- [axum](https://github.com/tokio-rs/axum) (MIT) - Web framework for building HTTP APIs and handling routing
- [hyper](https://github.com/hyperium/hyper) (MIT) - Low-level HTTP client/server implementation used by Axum
- [tonic](https://github.com/hyperium/tonic) (MIT) - gRPC framework for implementing gRPC services and clients
- [async-graphql](https://github.com/async-graphql/async-graphql) (MIT) - GraphQL framework for implementing GraphQL APIs and schema generation
- [paho-mqtt](https://github.com/eclipse/paho.mqtt.rust) (EPL/EDL) - MQTT client for handling MQTT protocol communication
- [ws](https://github.com/snapview/ws-rs) (MIT) - WebSocket library for handling WebSocket connections and messages
- [serde](https://github.com/serde-rs/serde) (MIT) - Serialization framework for converting between Rust types and various data formats
- [serde_json](https://github.com/serde-rs/json) (MIT) - JSON serialization support for Serde
- [prost](https://github.com/tokio-rs/prost) (MIT) - Protocol Buffers implementation for gRPC message serialization
- [config](https://github.com/mehcode/config-rs) (MIT) - Configuration management for loading and managing application settings
- [serde_yaml](https://github.com/dtolnay/serde-yaml) (MIT) - YAML serialization support for Serde, used for configuration files
- [toml](https://github.com/toml-rs/toml) (MIT) - TOML serialization support for Serde, used for configuration files
- [tracing](https://github.com/tokio-rs/tracing) (MIT) - Logging framework for structured logging and diagnostics
- [tracing-subscriber](https://github.com/tokio-rs/tracing) (MIT) - Logging subscriber implementation for the tracing framework
- [metrics](https://github.com/metrics-rs/metrics) (MIT) - Metrics framework for collecting and reporting application metrics
- [metrics-exporter-prometheus](https://github.com/metrics-rs/metrics) (MIT) - Prometheus metrics exporter for exposing metrics in Prometheus format
- [thiserror](https://github.com/dtolnay/thiserror) (MIT) - Error handling for creating custom error types with derive macros
- [anyhow](https://github.com/dtolnay/anyhow) (MIT) - Error handling for flexible error types and error propagation
- [async-trait](https://github.com/dtolnay/async-trait) (MIT) - Async trait support for implementing async traits
- [futures](https://github.com/rust-lang/futures-rs) (MIT) - Future utilities for working with async computations
- [tower](https://github.com/tower-rs/tower) (MIT) - Service middleware for building layered service architectures
- [tower-http](https://github.com/tower-rs/tower-http) (MIT) - HTTP middleware for Tower, providing HTTP-specific middleware components

## Development Dependencies

- [tokio-test](https://github.com/tokio-rs/tokio) (MIT) - Testing utilities for async code and Tokio runtime testing
- [wiremock](https://github.com/LukeMathWalker/wiremock-rs) (MIT) - HTTP mocking for testing HTTP interactions
- [tonic-build](https://github.com/hyperium/tonic) (MIT) - gRPC code generation for compiling Protocol Buffer definitions into Rust code 
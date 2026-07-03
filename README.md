# STM32 DevTools Package Lab

Portfolio project for developer-tooling work relevant to STMicroelectronics
STM32-style workflows. It demonstrates a Rust backend/core CLI, a TypeScript
client library, package manifest validation, dependency resolution, automated
tests, and Linux CI.

This project is not affiliated with STMicroelectronics. It is a learning and
portfolio lab built to practice the kind of software engineering used in
developer tools such as package managers, configuration tooling, and IDE
integration layers.

## What It Does

- Parses YAML or JSON package manifests for STM32-style development workspaces.
- Validates package metadata, semantic versions, MCU families, dependencies,
  artifacts, checksums, and toolchain configuration.
- Builds a deterministic installation plan using topological dependency order.
- Emits machine-readable JSON for integration with IDE extensions or internal
  APIs.
- Provides a TypeScript client for reading, validating, and summarizing the
  same manifest format.
- Runs automated Rust and TypeScript checks through GitHub Actions on Linux.

## Why This Project Exists

The target role focuses on software engineering for STM32 developer tools:
Rust/C++, JavaScript/TypeScript, backend components, package-management
infrastructure, internal APIs, Agile workflows, testing, maintainability, and
root-cause analysis. This lab is a compact implementation of those themes.

## Example Manifest

```yaml
workspace:
  name: motor-control-firmware
  target: stm32f407
  toolchain:
    compiler: arm-none-eabi-gcc
    version: "12.3.1"
packages:
  - name: cmsis-core
    version: "5.9.0"
    family: stm32f4
    source: registry
    artifact: cmsis-core-5.9.0.zip
    sha256: "7c52adf5dd2b1d6a5df9bfe709baedb08f1a65ccfe5f47ccdf67274d87f6d05d"
  - name: stm32f4-hal
    version: "1.8.0"
    family: stm32f4
    source: registry
    dependencies:
      - cmsis-core
    artifact: stm32f4-hal-1.8.0.zip
    sha256: "9301a2ffce0fdc5c8f1cf30500fda962e32ec9ee6c1f0b6562af8f0fd0608324"
```

## Rust CLI

```bash
cargo run -- validate examples/stm32f4-workspace.yml
cargo run -- plan examples/stm32f4-workspace.yml
cargo run -- inspect examples/stm32f4-workspace.yml
```

## TypeScript Client

```bash
cd clients/typescript
npm install
npm test
npm run build
```

## Repository Layout

```text
src/                  Rust manifest parser, validator, resolver, and CLI
examples/             Valid and intentionally faulty STM32-style manifests
clients/typescript/   TypeScript manifest client and tests
.github/workflows/    Linux CI for Rust and TypeScript checks
```

## Resume-Ready Summary

Built a Rust and TypeScript developer-tools lab for STM32-style package
management, implementing manifest parsing, semantic validation, dependency
resolution, JSON output, automated tests, and Linux GitHub Actions CI.


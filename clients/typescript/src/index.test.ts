import { describe, expect, it } from "vitest";
import { summarizeManifest, validateManifest, type Stm32PackageManifest } from "./index.js";

const manifest: Stm32PackageManifest = {
  workspace: {
    name: "motor-control-firmware",
    target: "stm32f407",
    toolchain: {
      compiler: "arm-none-eabi-gcc",
      version: "12.3.1"
    }
  },
  packages: [
    {
      name: "cmsis-core",
      version: "5.9.0",
      family: "stm32f4",
      source: "registry",
      artifact: "cmsis-core-5.9.0.zip",
      sha256: "7c52adf5dd2b1d6a5df9bfe709baedb08f1a65ccfe5f47ccdf67274d87f6d05d"
    },
    {
      name: "stm32f4-hal",
      version: "1.8.0",
      family: "stm32f4",
      source: "registry",
      dependencies: ["cmsis-core"],
      artifact: "stm32f4-hal-1.8.0.zip",
      sha256: "9301a2ffce0fdc5c8f1cf30500fda962e32ec9ee6c1f0b6562af8f0fd0608324"
    }
  ]
};

describe("validateManifest", () => {
  it("accepts a valid STM32-style manifest", () => {
    expect(validateManifest(manifest)).toEqual([]);
  });

  it("reports unknown dependencies", () => {
    const invalid: Stm32PackageManifest = {
      ...manifest,
      packages: [
        {
          ...manifest.packages[1],
          dependencies: ["missing-core"]
        }
      ]
    };

    expect(validateManifest(invalid)).toContain(
      "stm32f4-hal depends on unknown package missing-core"
    );
  });
});

describe("summarizeManifest", () => {
  it("summarizes package metadata for IDE or API integrations", () => {
    expect(summarizeManifest(manifest)).toEqual({
      workspace: "motor-control-firmware",
      target: "stm32f407",
      packageCount: 2,
      families: ["stm32f4"],
      dependencyEdges: 1
    });
  });
});


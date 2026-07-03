import { readFile } from "node:fs/promises";
import { extname } from "node:path";
import YAML from "yaml";

export type PackageSource = "registry" | "local-cache" | "git";

export interface Toolchain {
  compiler: string;
  version: string;
}

export interface Workspace {
  name: string;
  target: string;
  toolchain: Toolchain;
}

export interface PackageManifestEntry {
  name: string;
  version: string;
  family: string;
  source: PackageSource;
  dependencies?: string[];
  artifact: string;
  sha256: string;
}

export interface Stm32PackageManifest {
  workspace: Workspace;
  packages: PackageManifestEntry[];
}

export interface PackageSummary {
  workspace: string;
  target: string;
  packageCount: number;
  families: string[];
  dependencyEdges: number;
}

export async function readManifest(path: string): Promise<Stm32PackageManifest> {
  const raw = await readFile(path, "utf8");
  const extension = extname(path).toLowerCase();

  if (extension === ".json") {
    return JSON.parse(raw) as Stm32PackageManifest;
  }
  if (extension === ".yaml" || extension === ".yml") {
    return YAML.parse(raw) as Stm32PackageManifest;
  }

  throw new Error(`Unsupported manifest extension: ${extension}`);
}

export function validateManifest(manifest: Stm32PackageManifest): string[] {
  const errors: string[] = [];
  if (!manifest.workspace?.name?.trim()) {
    errors.push("workspace.name is required");
  }
  if (!/^stm32[a-z0-9]{3,}$/i.test(manifest.workspace?.target ?? "")) {
    errors.push("workspace.target must look like an STM32 target");
  }
  if (!manifest.workspace?.toolchain?.compiler?.trim()) {
    errors.push("workspace.toolchain.compiler is required");
  }
  if (!Array.isArray(manifest.packages) || manifest.packages.length === 0) {
    errors.push("packages must contain at least one package");
    return errors;
  }

  const packageNames = new Set<string>();
  for (const pkg of manifest.packages) {
    if (!pkg.name?.trim()) {
      errors.push("package.name is required");
      continue;
    }
    if (packageNames.has(pkg.name)) {
      errors.push(`duplicate package: ${pkg.name}`);
    }
    packageNames.add(pkg.name);
    if (!/^stm32[a-z0-9]{2,}$/i.test(pkg.family)) {
      errors.push(`invalid STM32 family for ${pkg.name}`);
    }
    if (!/^[a-f0-9]{64}$/.test(pkg.sha256)) {
      errors.push(`invalid sha256 for ${pkg.name}`);
    }
  }

  for (const pkg of manifest.packages) {
    for (const dependency of pkg.dependencies ?? []) {
      if (!packageNames.has(dependency)) {
        errors.push(`${pkg.name} depends on unknown package ${dependency}`);
      }
    }
  }

  return errors;
}

export function summarizeManifest(manifest: Stm32PackageManifest): PackageSummary {
  const families = [...new Set(manifest.packages.map((pkg) => pkg.family))].sort();
  const dependencyEdges = manifest.packages.reduce(
    (total, pkg) => total + (pkg.dependencies?.length ?? 0),
    0
  );

  return {
    workspace: manifest.workspace.name,
    target: manifest.workspace.target,
    packageCount: manifest.packages.length,
    families,
    dependencyEdges
  };
}


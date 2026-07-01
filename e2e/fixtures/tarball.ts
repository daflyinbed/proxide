const TARBALL_BASE64 =
  "H4sIAAAAAAAAE+2W32vbMBDH85y/QnjQp9qxLEeBMsbGlocNBmN7bFdQ5WuqxJaEpGQdo//79KPeQsnI" +
  "w5KUDX/9IOvurLuz/DHSjK/YAiY6jcXSKjk6sMqypHWNdtmD6hlBI0wqQmo8nVbVqMR4OsNoVB66kF1a" +
  "W8eML+Vv10m9oF/jP6IfY4QyyTrILlD2eqkcm+gVzpdrJrPz4NuAsULJ4MZFWdBkbcByI7R79CRjx0Sc" +
  "CdnAvf+SkjUFWu8IubzBgXUhDPidQlfZ3BhlLpBUKDiQ1cDFrYDmKkNnZwjuhUM4808+xNVW8P2bMk1Y" +
  "7vJrtLC1u1MmLPjBF40+Cc4ahV6GDmI/DWygVRpMwVX3KtXUCg7Sxp7ff3nbt6TBFy65gK1iffsN41yo" +
  "EHtdFbOiisWMH8bPvXUH0SP3k+KG3UBr+DFy7OGfEJr4x5iWVeS/pLQe+D+FIv/agIWI6GX66kFuIhT+" +
  "1gDjrp/4d7WAvAwEJPh0u14IufWkM0zaW2W6nLfM2lybgJ4LTJ0/jWiAK8OcMjt8MW3OlfQppcuhhQ6k" +
  "+2OgkK2Q8DssFPi/IHpU9fz3/+xj5NjDf8QFE39VmE4JDfzPCBn4P4X6/f88f/Pu47zomiPk2Lv/dOv8" +
  "h+P/34/D/p9CL+Kp67mrGDRo0KBBp9ZPsETQegASAAA=";

export const TARBALL_BYTES = Buffer.from(TARBALL_BASE64, "base64");

const TARBALL_LENGTH = 512;

export interface PublishPayloadOpts {
  description?: string;
  keywords?: string[];
  author?: string;
  access?: string;
}

export function buildPublishPayload(
  name: string,
  version: string,
  opts?: PublishPayloadOpts,
) {
  const tarballName = name.startsWith("@")
    ? `${name.split("/")[1]}-${version}.tgz`
    : `${name}-${version}.tgz`;

  const description = opts?.description ?? `e2e test package ${name}`;

  const versionEntry: Record<string, any> = {
    name,
    version,
    description,
    main: "index.js",
    scripts: { test: 'echo "ok"' },
    dist: {
      tarball: `http://localhost/-/${tarballName}`,
    },
    _id: `${name}@${version}`,
  };

  if (opts?.keywords) {
    versionEntry.keywords = opts.keywords;
  }
  if (opts?.author) {
    versionEntry.author = opts.author;
  }
  if (opts?.access) {
    versionEntry.publishConfig = { access: opts.access };
  }

  return {
    name,
    description,
    "dist-tags": { latest: version },
    versions: {
      [version]: versionEntry,
    },
    _attachments: {
      [tarballName]: {
        content_type: "application/octet-stream",
        data: TARBALL_BASE64,
        length: TARBALL_LENGTH,
      },
    },
  };
}

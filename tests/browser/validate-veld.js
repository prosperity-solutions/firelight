// D12: veld.json validates against Veld's published schema v3 (vendored as
// veld.schema.json, from github.com/prosperity-solutions/veld, MIT).
const fs = require("node:fs");
const path = require("node:path");
const Ajv2020 = require("ajv/dist/2020").default;
const schema = JSON.parse(fs.readFileSync(path.join(__dirname, "veld.schema.json"), "utf8"));
const config = JSON.parse(fs.readFileSync(path.join(__dirname, "../../veld.json"), "utf8"));
const ajv = new Ajv2020({ allErrors: true, strict: false });
const validate = ajv.compile(schema);
if (!validate(config)) {
  console.error("veld.json does not match Veld's schema v3:");
  for (const e of validate.errors) console.error(`  ${e.instancePath || "/"} ${e.message}`);
  process.exit(1);
}
console.log("veld.json matches Veld's schema v3.");

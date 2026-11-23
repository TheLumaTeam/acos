import { ACOSClient } from "../src/index.ts";

const token = process.env.ACOS_TOKEN;
if (!token) throw new Error("Set ACOS_TOKEN before running this example");

const client = new ACOSClient({ token });
const computer = await client.createComputer({
  name: "example-agent",
  kind: "vm",
  resources: { vcpus: 2, memory_mib: 2048 },
  image: "acos:latest",
});
console.log(await client.startComputer(computer.id));
console.log(await client.getComputer(computer.id));
console.log(await client.stopComputer(computer.id));
await client.deleteComputer(computer.id);

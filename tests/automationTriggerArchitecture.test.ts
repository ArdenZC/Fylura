import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
const AUTOMATION_TRIGGER_SCHEMA_MINIMUM_VERSION = 37;
const read=(path:string)=>readFileSync(path,"utf8");
const readSchemaVersionAtLeast = (schema: string, minimum: number): number => {
 const declarations = [...schema.matchAll(/^[ \t]*pub\(crate\)[ \t]+const[ \t]+CURRENT_SCHEMA_VERSION:[ \t]*i32[ \t]*=[ \t]*([^;\r\n]*)[ \t]*;[ \t]*$/gm)];
 if (declarations.length !== 1) throw new Error("Expected exactly one CURRENT_SCHEMA_VERSION i32 declaration");
 const literal = declarations[0]?.[1];
 if (literal === undefined || !/^[+-]?\d(?:_?\d)*$/.test(literal)) throw new Error("CURRENT_SCHEMA_VERSION must be an integer literal");
 const version = Number(literal.replaceAll("_", ""));
 if (!Number.isSafeInteger(version) || version < -2_147_483_648 || version > 2_147_483_647) throw new Error("CURRENT_SCHEMA_VERSION must fit in an i32");
 if (version < minimum) throw new Error(`Automation Trigger requires schema version ${minimum} or later`);
 return version;
};
it("PM-02B retains one wake/deadline coordinator and existing Background admission",()=>{
 const runtime=read("src-tauri/src/db/automation/runtime.rs");
 expect(runtime.match(/\.spawn\(/g)).toHaveLength(1);
 expect(runtime).toContain("Condvar"); expect(runtime).toContain("wait_while");
 expect(runtime).toContain("WorkClass::Background"); expect(runtime).toContain("acquire_with_backpressure");
 expect(runtime).not.toMatch(/thread::sleep|interval|tokio::time|NotifyWatcher|ReadDirectoryChanges/);
 for(const path of ["runtime.rs","calendar.rs","trigger_state.rs"]){
  expect(read(`src-tauri/src/db/automation/${path}`)).not.toMatch(/execute_organization_plan|dry_run_organization|update_organization_plan_decisions|Command::new|reqwest|create_dir|remove_file|fs::write/);
 }
 expect(read("src/views/automation/useAutomationIntents.ts")).toContain("onAutomationUpdated");
 expect(read("src/views/automation/useAutomationIntents.ts")).not.toMatch(/setInterval|setTimeout/);
});
it("PM-02B has exactly schema37 and native resume hints without polling",()=>{
 expect(readSchemaVersionAtLeast(read("src-tauri/src/db/schema.rs"), AUTOMATION_TRIGGER_SCHEMA_MINIMUM_VERSION)).toBeGreaterThanOrEqual(AUTOMATION_TRIGGER_SCHEMA_MINIMUM_VERSION);
 const adapter=read("src-tauri/src/platform/windows/automation_resume.rs");
 expect(adapter).toContain("PowerRegisterSuspendResumeNotification");
 expect(adapter).toContain("PowerUnregisterSuspendResumeNotification");
 expect(adapter).not.toMatch(/thread::spawn|sleep\(|SetWaitableTimer|SELECT|UPDATE|INSERT/);
});
it("accepts later schemas and rejects invalid versions below the PM-02B migration floor",()=>{
 const schemaWithVersion=(version:string)=>`pub(crate) const CURRENT_SCHEMA_VERSION: i32 = ${version};`;
 expect(readSchemaVersionAtLeast(schemaWithVersion("39"), AUTOMATION_TRIGGER_SCHEMA_MINIMUM_VERSION)).toBe(39);
 expect(()=>readSchemaVersionAtLeast(schemaWithVersion("36"), AUTOMATION_TRIGGER_SCHEMA_MINIMUM_VERSION)).toThrow(/schema version 37 or later/);
 expect(()=>readSchemaVersionAtLeast("", AUTOMATION_TRIGGER_SCHEMA_MINIMUM_VERSION)).toThrow(/exactly one/);
 expect(()=>readSchemaVersionAtLeast(`${schemaWithVersion("38")}\n${schemaWithVersion("39")}`, AUTOMATION_TRIGGER_SCHEMA_MINIMUM_VERSION)).toThrow(/exactly one/);
 for(const invalid of ["not-a-number","2147483648","-2147483649"]){
  expect(()=>readSchemaVersionAtLeast(schemaWithVersion(invalid), AUTOMATION_TRIGGER_SCHEMA_MINIMUM_VERSION)).toThrow();
 }
});

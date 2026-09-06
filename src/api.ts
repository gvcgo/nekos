import { invoke } from "@tauri-apps/api/core";

/** Thin typed bridge to the Rust orchestrator commands (src-tauri/src/lib.rs). */

export function ping(): Promise<string> {
  return invoke<string>("ping");
}

/** Report the embedded sing-box version as seen by the core process. */
export function coreVersion(): Promise<string> {
  return invoke<string>("core_version");
}

export interface CoreStatus {
  running: boolean;
  started_at?: string;
}

export function coreStatus(): Promise<CoreStatus> {
  return invoke<CoreStatus>("core_status");
}

/** Forward raw text (links/subscription payload) to the core parser. */
export interface ParseError {
  line: number;
  snippet: string;
  reason: string;
}

export interface NodeMeta {
  id: string;
  remark: string;
  type: string;
}

export interface ImportResult {
  nodes: NodeMeta[];
  errors: ParseError[];
}

export function parseText(text: string): Promise<ImportResult> {
  return invoke<ImportResult>("parse_text", { text });
}

<script setup lang="ts">
import { onMounted, onUnmounted, ref, watch } from "vue";
import { logTail, type LogEntry } from "../api";

type LevelFilter = "all" | "error" | "warn" | "info" | "debug";

const entries = ref<LogEntry[]>([]);
const filter = ref<LevelFilter>("all");
const autoscroll = ref(true);
const paused = ref(false);
const err = ref("");
const area = ref<HTMLElement | null>(null);

const FILTERS: { id: LevelFilter; label: string }[] = [
  { id: "all", label: "全部" },
  { id: "error", label: "错误" },
  { id: "warn", label: "警告" },
  { id: "info", label: "信息" },
  { id: "debug", label: "调试" },
];

const counts = ref<Record<string, number>>({});

function levelClass(level: string): string {
  return level === "other" ? "info" : level;
}

function matches(e: LogEntry): boolean {
  if (filter.value === "all") return true;
  return e.level === filter.value || (filter.value === "error" && e.level === "error");
}

const visible = () => entries.value.filter(matches);

async function poll() {
  if (paused.value) return;
  try {
    const got = await logTail(600);
    if (entries.value.length && entries.value[entries.value.length - 1].line === got[got.length - 1]?.line && entries.value.length === got.length) {
      return; // unchanged
    }
    entries.value = got;
    counts.value = {};
    for (const e of got) counts.value[e.level] = (counts.value[e.level] ?? 0) + 1;
    if (autoscroll.value) {
      // scroll after paint
      requestAnimationFrame(() => {
        const el = area.value;
        if (el) el.scrollTop = el.scrollHeight;
      });
    }
  } catch (e) {
    err.value = String(e);
  }
}

let timer: number | undefined;
onMounted(() => {
  poll();
  timer = window.setInterval(poll, 1500);
});
onUnmounted(() => {
  if (timer) window.clearInterval(timer);
});
</script>

<template>
  <div class="logs-page">
    <div class="toolbar">
      <h1>日志</h1>
      <span class="hint">sing-box core 输出（重启内核后新会话日志从头记录）</span>
      <span class="spacer"></span>
      <button v-for="f in FILTERS" :key="f.id" class="chip-btn" :class="{ active: filter === f.id }" @click="filter = f.id">
        {{ f.label }}<span v-if="f.id !== 'all' && counts[f.id]" class="cnt">{{ counts[f.id] }}</span>
      </button>
      <label class="toggle">
        <input v-model="autoscroll" type="checkbox" /> 自动滚动
      </label>
      <label class="toggle">
        <input v-model="paused" type="checkbox" /> 暂停
      </label>
    </div>

    <div ref="area" class="log-area">
      <div v-if="err" class="err">{{ err }}</div>
      <div v-if="!entries.length && !err" class="empty">暂无日志 — 启动代理后这里会实时显示 sing-box 输出。</div>
      <div v-for="(e, i) in visible()" :key="i" class="line" :class="levelClass(e.level)">
        <span class="lvl">{{ e.level }}</span>
        <span class="txt">{{ e.line }}</span>
      </div>
    </div>
  </div>
</template>

<style scoped>
.logs-page { display: flex; flex-direction: column; gap: 10px; height: calc(100vh - 90px); min-height: 320px; }
.toolbar { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
.toolbar h1 { margin: 0 8px 0 0; font-size: 20px; }
.hint { opacity: 0.6; font-size: 12px; }
.spacer { flex: 1; }
.chip-btn {
  border: 1px solid var(--border); border-radius: 14px; background: transparent;
  color: inherit; font-size: 12px; padding: 3px 10px; cursor: pointer;
}
.chip-btn.active { background: var(--accent); border-color: var(--accent); color: #fff; }
.cnt { margin-left: 4px; opacity: 0.7; font-size: 11px; }
.toggle { display: inline-flex; align-items: center; gap: 4px; font-size: 12px; cursor: pointer; }
.log-area {
  flex: 1; overflow: auto; border: 1px solid var(--border); border-radius: 8px;
  font-family: ui-monospace, "Cascadia Mono", monospace; font-size: 12px; padding: 6px 0;
  min-height: 0;
}
.line { display: flex; gap: 8px; padding: 1px 10px; line-height: 1.55; }
.line .lvl { width: 46px; flex: none; text-transform: uppercase; font-size: 10px; letter-spacing: 0.4px; }
.line.error .lvl { color: #ef4444; }
.line.error .txt { color: #ef4444; }
.line.warn .lvl { color: #f59e0b; }
.line.warn .txt { color: #fbbf24; }
.line.info .lvl { color: #60a5fa; }
.line.debug .lvl { color: #9ca3af; }
.line .txt { white-space: pre-wrap; word-break: break-all; }
.err { color: #ef4444; font-size: 12px; padding: 8px 10px; }
.empty { padding: 20px; text-align: center; opacity: 0.55; }
</style>

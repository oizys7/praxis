import { invoke } from "@tauri-apps/api/core";
import { normalizeError } from "./useAgent";
import type {
  ChapterAnnotation,
  ChapterNote,
  ChatMessage,
  PlanResponse,
} from "@/types/agent";

/**
 * useStudy：研读页（2026-11 功能包1）后端命令的调用层。
 *
 * 与 usePlanHistory 同构：模块级独立 async 函数、无共享状态、错误统一经
 * normalizeError 归一为 CommandError。事件类数据（chapter-delta /
 * chat-delta / chat-done / chat-error）由页面组件经 @tauri-apps/api/event
 * 直接订阅，不在此封装（页面自管订阅生命周期）。
 */

/** 按需生成空章节正文（复用 regen 通道）；正文增量经 'chapter-delta' 全局事件推送 */
export async function generateChapterContent(
  planId: string,
  phaseIndex: number,
  chapterIndex: number,
): Promise<PlanResponse> {
  try {
    return await invoke<PlanResponse>("generate_chapter_content", {
      planId,
      phaseIndex,
      chapterIndex,
    });
  } catch (e) {
    throw normalizeError(e);
  }
}

/** 读取本章聊天记录（seq 升序） */
export async function fetchChatMessages(
  planId: string,
  phaseIndex: number,
  chapterIndex: number,
): Promise<ChatMessage[]> {
  try {
    return await invoke<ChatMessage[]>("list_chat_messages", {
      planId,
      phaseIndex,
      chapterIndex,
    });
  } catch (e) {
    throw normalizeError(e);
  }
}

/** 研读伴学聊天（NDJSON 流式）：回答增量经 'chat-delta'，终态经 chat-done/chat-error */
export async function sendChatMessage(
  planId: string,
  phaseIndex: number,
  chapterIndex: number,
  question: string,
): Promise<void> {
  try {
    await invoke("chat_stream", {
      planId,
      phaseIndex,
      chapterIndex,
      question,
    });
  } catch (e) {
    throw normalizeError(e);
  }
}

/** 读取本章全部标注（创建时间升序） */
export async function fetchAnnotations(
  planId: string,
  phaseIndex: number,
  chapterIndex: number,
): Promise<ChapterAnnotation[]> {
  try {
    return await invoke<ChapterAnnotation[]>("list_annotations", {
      planId,
      phaseIndex,
      chapterIndex,
    });
  } catch (e) {
    throw normalizeError(e);
  }
}

/** 保存选区标注（id 由前端生成；同 id 重复保存 = 编辑回写） */
export async function saveAnnotation(
  planId: string,
  annotation: ChapterAnnotation,
): Promise<void> {
  try {
    await invoke("save_annotation", { planId, annotation });
  } catch (e) {
    throw normalizeError(e);
  }
}

/** 删除选区标注（幂等） */
export async function deleteAnnotation(
  planId: string,
  phaseIndex: number,
  chapterIndex: number,
  annotationId: string,
): Promise<void> {
  try {
    await invoke("delete_annotation", {
      planId,
      phaseIndex,
      chapterIndex,
      annotationId,
    });
  } catch (e) {
    throw normalizeError(e);
  }
}

/** 读取章节笔记（一章至多一行；无笔记返回 null） */
export async function fetchChapterNote(
  planId: string,
  phaseIndex: number,
  chapterIndex: number,
): Promise<ChapterNote | null> {
  try {
    return await invoke<ChapterNote | null>("get_chapter_note", {
      planId,
      phaseIndex,
      chapterIndex,
    });
  } catch (e) {
    throw normalizeError(e);
  }
}

/** 保存章节笔记（upsert；contentMd 空串 = 清空） */
export async function saveChapterNote(
  planId: string,
  phaseIndex: number,
  chapterIndex: number,
  contentMd: string,
): Promise<void> {
  try {
    await invoke("save_chapter_note", {
      planId,
      phaseIndex,
      chapterIndex,
      contentMd,
    });
  } catch (e) {
    throw normalizeError(e);
  }
}

/** 前端生成标注 id（端特有字段；Rust 落库以透传 id 为主键） */
export function newAnnotationId(): string {
  return crypto.randomUUID();
}

/**
 * The IPC surface for Phase 8: smart mailboxes, rules, flags, VIPs, junk, snooze and undo.
 *
 * Kept out of `ipc.ts` only because that file is already long; the same rules apply. Every
 * function has a browser path, and where the browser cannot answer honestly it returns the
 * empty result rather than an invented one. A browser fallback that fabricates plausible rules
 * would make the gallery look finished while proving nothing.
 */

import { invoke, isTauri } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

import type { Action } from './generated/Action'
import type { Available } from './generated/Available'
import type { FlagName } from './generated/FlagName'
import type { JunkStatus } from './generated/JunkStatus'
import type { MessageRow } from './generated/MessageRow'
import type { Predicate } from './generated/Predicate'
import type { Rule } from './generated/Rule'
import type { RunReport } from './generated/RunReport'
import type { SmartMailbox } from './generated/SmartMailbox'
import type { Vip } from './generated/Vip'

const inTauri: boolean = isTauri()

/* ------------------------------------------------------------------ smart mailboxes */

export async function smartList(): Promise<SmartMailbox[]> {
  if (!inTauri) return []
  return invoke<SmartMailbox[]>('smart_list')
}

export async function smartSave(
  id: number | null,
  name: string,
  icon: string | null,
  predicate: Predicate,
): Promise<number> {
  return invoke<number>('smart_save', { id, name, icon, predicate })
}

export async function smartDelete(id: number): Promise<void> {
  await invoke('smart_delete', { id })
}

export async function smartMessages(
  predicate: Predicate,
  limit: number,
  offset: number,
): Promise<MessageRow[]> {
  if (!inTauri) return []
  return invoke<MessageRow[]>('smart_messages', { predicate, limit, offset })
}

/* ------------------------------------------------------------------------- rules */

export async function rulesList(): Promise<Rule[]> {
  if (!inTauri) return []
  return invoke<Rule[]>('rules_list')
}

export async function ruleSave(
  id: number | null,
  name: string,
  enabled: boolean,
  predicate: Predicate,
  actions: Action[],
): Promise<number> {
  return invoke<number>('rule_save', { id, name, enabled, predicate, actions })
}

export async function ruleDelete(id: number): Promise<void> {
  await invoke('rule_delete', { id })
}

/** Alt+Ctrl+L. Runs the rules over the selection, not the whole mailbox. */
export async function rulesRun(ids: number[]): Promise<RunReport> {
  return invoke<RunReport>('rules_run', { ids })
}

/* -------------------------------------------------------------------------- flags */

export async function flagNames(): Promise<FlagName[]> {
  if (!inTauri) return []
  return invoke<FlagName[]>('flag_names')
}

export async function flagRename(color: string, name: string): Promise<void> {
  await invoke('flag_rename', { color, name })
}

/** `null` clears the flag entirely. */
export async function flagSet(ids: number[], color: string | null): Promise<number> {
  return invoke<number>('flag_set', { ids, color })
}

/* --------------------------------------------------------------------------- VIPs */

export async function vipsList(): Promise<Vip[]> {
  if (!inTauri) return []
  return invoke<Vip[]>('vips_list')
}

export async function vipAdd(address: string): Promise<string> {
  return invoke<string>('vip_add', { address })
}

export async function vipRemove(address: string): Promise<void> {
  await invoke('vip_remove', { address })
}

/* --------------------------------------------------------------------------- junk */

export async function junkStatus(): Promise<JunkStatus> {
  if (!inTauri) {
    return { ready: false, cleanExamples: 0, junkExamples: 0, needed: 0 }
  }
  return invoke<JunkStatus>('junk_status')
}

export async function junkMark(ids: number[], isJunk: boolean): Promise<number> {
  return invoke<number>('junk_mark', { ids, isJunk })
}

export async function junkScan(mailboxId: number): Promise<number> {
  return invoke<number>('junk_scan', { mailboxId })
}

export async function blockedList(): Promise<string[]> {
  if (!inTauri) return []
  return invoke<string[]>('blocked_list')
}

export async function blockSender(address: string): Promise<void> {
  await invoke('block_sender', { address })
}

export async function unblockSender(address: string): Promise<void> {
  await invoke('unblock_sender', { address })
}

/* ------------------------------------------------------------ Remind Me and muting */

/**
 * `until` is absolute, in seconds since the epoch, and is computed here rather than in the
 * core. "Tomorrow morning" is a question about the user's timezone and their idea of morning,
 * and the core knows neither.
 */
export async function snooze(ids: number[], until: number): Promise<number> {
  return invoke<number>('snooze', { request: { ids, until } })
}

export async function unsnooze(ids: number[]): Promise<number> {
  return invoke<number>('unsnooze', { ids })
}

export async function muteThread(threadId: number, muted: boolean): Promise<void> {
  await invoke('mute_thread', { threadId, muted })
}

export async function detectFollowUps(): Promise<number> {
  if (!inTauri) return 0
  return invoke<number>('follow_ups_detect')
}

/* --------------------------------------------------------------------------- undo */

export async function undoAvailable(): Promise<Available> {
  if (!inTauri) return { undo: null, redo: null }
  return invoke<Available>('undo_available')
}

export async function performUndo(): Promise<string | null> {
  return invoke<string | null>('undo_perform')
}

export async function performRedo(): Promise<string | null> {
  return invoke<string | null>('redo_perform')
}

/* -------------------------------------------------------------------------- events */

/**
 * Subscribes to one of the core's change events.
 *
 * The UI invalidates on these rather than polling (standing rule 14). Returning a no-op
 * unsubscribe in the browser keeps every caller's cleanup path identical.
 */
export async function onChanged(event: OrganiseEvent, handler: () => void): Promise<UnlistenFn> {
  if (!inTauri) return () => undefined
  return listen(event, () => {
    handler()
  })
}

export type OrganiseEvent =
  | 'smart:changed'
  | 'rules:changed'
  | 'flags:changed'
  | 'vips:changed'
  | 'junk:changed'
  | 'blocked:changed'
  | 'mailbox:changed'

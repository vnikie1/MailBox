/**
 * Organising mail: rules, smart mailboxes, flags, reminders and undo. docs/01 §8.
 *
 * One feature folder rather than five, because these all sit on the same predicate engine in
 * the core and share a condition editor. Splitting them would mean four copies of that editor
 * drifting apart.
 */

export { ActionEditor, type ActionEditorProps } from './ActionEditor'
export { FlagMenu, type FlagMenuProps } from './FlagMenu'
export { PredicateEditor, type PredicateEditorProps } from './PredicateEditor'
export { BLANK_CONDITION, fromFlat, opsFor, takesValue, toFlat } from './predicateShape'
export { MailboxPicker, type MailboxPickerProps } from './MailboxPicker'
export { JunkSettings } from './JunkSettings'
export { OrganiseSettings } from './OrganiseSettings'
export { RemindMenu, type RemindMenuProps } from './RemindMenu'
export { SmartMailboxEditor, type SmartMailboxEditorProps } from './SmartMailboxEditor'
export { useOrganiseShortcuts, type OrganiseShortcuts } from './useOrganiseShortcuts'
export { RulesEditor, type RulesEditorProps } from './RulesEditor'
export { useUndo } from './useUndo'

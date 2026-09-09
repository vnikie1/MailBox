import { useQueryClient } from '@tanstack/react-query'
import { useCallback, useEffect, useMemo, useState } from 'react'
import { Download, FolderOpen, Upload } from 'lucide-react'

import { keys, useMailboxes } from '@/app/queries'
import type { ImportSource } from '@/lib/generated/ImportSource'
import type { TransferProgress } from '@/lib/generated/TransferProgress'
import {
  exportPickFolder,
  exportRun,
  importPickFiles,
  importRun,
  importSources,
  onTransferProgress,
  runningInTauri,
} from '@/lib/ipc'
import { Button, Select, type SelectOption } from '@/ui'

import { Field, Form } from './SettingsForm'
import styles from './settings.module.css'
import pane from './TransferSettings.module.css'

/**
 * Settings → Advanced → Import and export. docs/06 Phase 11.
 *
 * ## Why it is here rather than in a File menu
 *
 * Mail puts import under File ▸ Import Mailboxes. This app has no menu bar and will not get
 * one: Windows owns the caption strip (see `platform/mod.rs`), and adding an in-window menu
 * bar to get one command would cost a band of chrome across every window for the rest of the
 * app's life. Settings is the only chrome there is, and Advanced is where the machinery lives.
 *
 * ## What it says before it writes
 *
 * That importing twice duplicates. There is no stable identity to match an mbox message
 * against — `Message-ID` is missing from a lot of old mail and forged in some of the rest — so
 * a second import of the same file genuinely does add every message again. Telling the user
 * afterwards would be telling them after they had to fix it.
 */

/** A byte count, for a folder list where the useful signal is "big" versus "small". */
function formatSize(bytes: number): string {
  if (bytes < 1024) return `${String(bytes)} B`
  if (bytes < 1024 * 1024) return `${String(Math.round(bytes / 1024))} KB`
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}

/**
 * "3 messages in 1 mailbox", not "3 messages in 1 mailboxes".
 *
 * The import of a single mbox file is the most common case there is, so the one number most
 * likely to be 1 was the one being printed wrong every time.
 */
function count(n: number, singular: string, plural = `${singular}s`): string {
  return `${String(n)} ${n === 1 ? singular : plural}`
}

/**
 * The two export formats.
 *
 * The name of the format is the option; what it is *for* is the hint underneath. As two
 * radios, each label carried its own explanation — "One mbox file per mailbox — for
 * Thunderbird, Apple Mail and most other programs" — which is a sentence pretending to be a
 * label, and it wrapped to two lines in a settings window this narrow.
 */
const FORMATS: SelectOption<'mbox' | 'eml'>[] = [
  { value: 'mbox', label: 'One mbox file per mailbox' },
  { value: 'eml', label: 'A folder of .eml files' },
]

const FORMAT_HINTS: Record<'mbox' | 'eml', string> = {
  mbox: 'Read by Thunderbird, Apple Mail and most other programs.',
  eml: 'One file per message, readable in Windows and Outlook.',
}

export function TransferSettings() {
  const [sources, setSources] = useState<ImportSource[] | null>(null)
  const [chosen, setChosen] = useState<Set<string>>(new Set())
  const [progress, setProgress] = useState<TransferProgress | null>(null)
  const [format, setFormat] = useState<'mbox' | 'eml'>('mbox')

  const { data: mailboxes = [] } = useMailboxes()
  const client = useQueryClient()

  useEffect(() => {
    void importSources().then(setSources)
  }, [])

  useEffect(() => {
    let cancelled = false
    let stop: (() => void) | undefined

    void onTransferProgress((update) => {
      setProgress(update)

      // An import creates a mailbox and an account that were not there when this window
      // opened, and `mailboxes` is what "Export all mail" iterates. Without this, importing
      // and then exporting in the same sitting writes a backup that is silently missing the
      // mail just imported -- no error, no warning, just one file fewer than there are
      // mailboxes. Observed: 46 mailboxes in the database, 45 files written, and the same
      // export from a freshly opened Settings window produced all 46.
      if (update.finished !== null) {
        void client.invalidateQueries({ queryKey: keys.mailboxes })
      }
    }).then((unlisten) => {
      if (cancelled) unlisten()
      else stop = unlisten
    })

    return () => {
      cancelled = true
      stop?.()
    }
  }, [client])

  const running = progress !== null && progress.finished === null

  // Every folder across every profile, flattened — the file path is the identity, because two
  // profiles can both have a folder called Inbox.
  const folders = useMemo(
    () =>
      (sources ?? []).flatMap((source) =>
        source.folders.map((folder) => ({ ...folder, profile: source.name })),
      ),
    [sources],
  )

  // Folders that exist and cannot be read, so "nothing found" and "nothing readable" can be
  // told apart in the message below.
  const maildirFolders = useMemo(
    () => (sources ?? []).reduce((total, source) => total + source.maildirFolders, 0),
    [sources],
  )

  const toggle = useCallback((file: string) => {
    setChosen((current) => {
      const next = new Set(current)
      if (next.has(file)) next.delete(file)
      else next.add(file)
      return next
    })
  }, [])

  const startImport = () => {
    const requests = folders
      .filter((folder) => chosen.has(folder.file))
      .map((folder) => ({ path: `${folder.profile}/${folder.path}`, file: folder.file }))

    if (requests.length === 0) return
    setProgress({ label: '', done: 0, total: requests.length, messages: 0, finished: null })
    void importRun(requests)
  }

  const startFileImport = () => {
    void importPickFiles().then((files) => {
      if (files.length === 0) return

      const requests = files.map((file) => ({
        // The file's own name is the mailbox name. A loose mbox carries no folder structure —
        // a .pst does, and the core ignores this for one and uses the tree inside the file.
        path:
          file
            .split(/[\\/]/)
            .pop()
            ?.replace(/\.(mbox|pst)$/i, '') ?? 'Imported',
        file,
      }))

      setProgress({ label: '', done: 0, total: requests.length, messages: 0, finished: null })
      void importRun(requests)
    })
  }

  const startExport = () => {
    void exportPickFolder().then((directory) => {
      if (directory === null) return

      const ids = mailboxes.map((mailbox) => mailbox.id)
      if (ids.length === 0) return

      setProgress({ label: '', done: 0, total: ids.length, messages: 0, finished: null })
      void exportRun(ids, format, directory)
    })
  }

  return (
    <>
      <section className={styles.section}>
        <h2 className={styles.heading}>Import</h2>

        <Form>
          <Field
            label="From Thunderbird"
            hint={
              folders.length === 0
                ? undefined
                : 'Everything you choose is copied into a local account called “On My PC”. Your existing accounts are not touched.'
            }
          >
            {!runningInTauri ? (
              <p className={styles.hint}>
                Importing reads files from this machine, so it needs the app.
              </p>
            ) : sources === null ? (
              <p className={styles.hint}>Looking…</p>
            ) : folders.length === 0 && maildirFolders > 0 ? (
              // Not the same thing as finding nothing, and the user was being told it was. A
              // profile stored as maildir — one file per message rather than one per folder —
              // produced an empty folder list, which reads as "you have no mail" to somebody
              // looking at a perfectly good archive.
              <p className={styles.hint}>
                Found, but its mail is stored one file per message
                {maildirFolders === 1 ? ' in 1 folder' : ` in ${String(maildirFolders)} folders`},
                which Halcyon cannot read yet. In Thunderbird, Account Settings → Server Settings →
                Message Store Type can be switched to &ldquo;File per folder (mbox)&rdquo;.
              </p>
            ) : folders.length === 0 ? (
              <p className={styles.hint}>None found on this machine.</p>
            ) : (
              <>
                <ul className={pane.folders}>
                  {folders.map((folder) => (
                    <li key={folder.file}>
                      <label className={styles.choice}>
                        <input
                          type="checkbox"
                          className={styles.checkbox}
                          checked={chosen.has(folder.file)}
                          disabled={running}
                          onChange={() => {
                            toggle(folder.file)
                          }}
                        />
                        <span className={pane.folderName}>
                          {folder.profile}/{folder.path}
                        </span>
                        <span className={styles.name}>{formatSize(folder.bytes)}</span>
                      </label>
                    </li>
                  ))}
                </ul>

                <Button
                  variant="bordered"
                  disabled={running || chosen.size === 0}
                  onClick={startImport}
                >
                  <Download size={16} aria-hidden />
                  Import {chosen.size > 0 ? `${String(chosen.size)} folders` : 'selected'}
                </Button>
              </>
            )}
          </Field>

          {/* Six lines of prose about .pst became two. What was cut was the reassurance —
              which folders, dates and senders survive — and what was kept is the part
              somebody has to know *before* choosing a file: attachments do not come across. */}
          <Field
            label="From a file"
            hint="An mbox file, or an Outlook .pst. From a .pst, folders, dates, senders and read state come across; attachments do not, and a few older messages arrive without their body. Both are counted when the import finishes."
          >
            <Button variant="bordered" disabled={running} onClick={startFileImport}>
              <FolderOpen size={16} aria-hidden />
              Choose files…
            </Button>
          </Field>

          <Field hint="Importing the same mail twice adds it twice — there is nothing in an mbox file that reliably identifies a message, so nothing can tell it has seen one before." />
        </Form>
      </section>

      <section className={styles.section}>
        <h2 className={styles.heading}>Export</h2>

        <Form>
          <Field label="Save as" htmlFor="export-format" hint={FORMAT_HINTS[format]}>
            <Select
              id="export-format"
              label="Save as"
              hideLabel
              options={FORMATS}
              value={format}
              disabled={running}
              onValueChange={setFormat}
            />
          </Field>

          <Field hint="Only messages that have been downloaded can be exported. A message whose contents were never fetched is an entry in a list and nothing more.">
            <Button
              variant="bordered"
              disabled={running || mailboxes.length === 0}
              onClick={startExport}
            >
              <Upload size={16} aria-hidden />
              Export all mail…
            </Button>
          </Field>

          {/* Reserved rather than appearing, because it reports both jobs and sits directly
              under the buttons that start them. Standing rule 6. */}
          <Field label={progress === null ? undefined : 'Progress'}>
            <p className={styles.status} aria-live="polite">
              {progress === null
                ? ''
                : progress.finished === null
                  ? `Working… ${String(progress.done)} of ${String(progress.total)}${
                      progress.label === '' ? '' : ` — ${progress.label}`
                    }`
                  : progress.finished.error !== null
                    ? `Finished with a problem: ${progress.finished.error}. ${String(
                        progress.finished.messages,
                      )} messages were handled.`
                    : `Done. ${count(progress.finished.messages, 'message')} in ${count(
                        progress.finished.folders,
                        'mailbox',
                        'mailboxes',
                      )}${
                        progress.finished.skipped > 0
                          ? `, ${String(progress.finished.skipped)} skipped`
                          : ''
                      }.`}
            </p>
          </Field>
        </Form>
      </section>
    </>
  )
}

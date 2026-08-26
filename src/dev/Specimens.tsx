import { useState, type ReactNode } from 'react'
import {
  Archive,
  Check,
  CornerUpLeft,
  Flag,
  FolderInput,
  Forward,
  PanelLeft,
  Reply,
  Search,
  Trash2,
  Undo2,
} from 'lucide-react'

import {
  Avatar,
  Badge,
  Button,
  Chip,
  ContextMenu,
  Divider,
  IconButton,
  Menu,
  MenuItem,
  MenuSection,
  MenuSeparator,
  Popover,
  ScrollArea,
  Sheet,
  Skeleton,
  TextField,
  Toast,
  TokenField,
  Tooltip,
  TooltipGroup,
  useToast,
  type Token,
} from '@/ui'

import styles from './Gallery.module.css'

interface SectionProps {
  title: string
  children: ReactNode
}

function Section({ title, children }: SectionProps) {
  return (
    <section className={styles.section}>
      <h2 className={styles.sectionTitle}>{title}</h2>
      <div className={styles.specimens}>{children}</div>
    </section>
  )
}

interface RowProps {
  label: string
  children: ReactNode
}

function Row({ label, children }: RowProps) {
  return (
    <div className={styles.row}>
      <span className={styles.rowLabel}>{label}</span>
      <div className={styles.rowItems}>{children}</div>
    </div>
  )
}

const SAMPLE_MENU = (
  <>
    <MenuItem label="Reply" icon={Reply} shortcut="Ctrl+R" />
    <MenuItem label="Reply All" icon={CornerUpLeft} shortcut="Ctrl+Shift+R" />
    <MenuItem label="Forward" icon={Forward} shortcut="Ctrl+Shift+F" />
    <MenuSeparator />
    <MenuItem label="Mark as Unread" checked={false} shortcut="Ctrl+U" />
    <MenuItem label="Flag" checked icon={Flag} shortcut="Ctrl+L" />
    <MenuSeparator />
    <Menu label="Move to" icon={FolderInput}>
      <MenuItem label="Archive" icon={Archive} />
      <MenuItem label="Receipts" />
      <Menu label="Projects">
        <MenuItem label="Halcyon" />
        <MenuItem label="Halyard" />
      </Menu>
    </Menu>
    <MenuItem label="Redirect" disabled shortcut="Ctrl+Shift+E" />
    <MenuSeparator />
    <MenuItem label="Delete" icon={Trash2} shortcut="Delete" destructive />
  </>
)

/**
 * Every primitive in every state.
 *
 * Rendered twice by the gallery — once forced light, once forced dark — so a single
 * screenshot is evidence for the "both themes ship together" half of standing rule 8.
 * It is also the Playwright visual baseline, so nothing in here may be random, timed or
 * dependent on the clock.
 */
export function Specimens() {
  const [text, setText] = useState('')
  const [filled, setFilled] = useState('ada@example.com')
  const [search, setSearch] = useState('')
  const [sheetOpen, setSheetOpen] = useState(false)
  const [flagged, setFlagged] = useState(true)
  const [tokens, setTokens] = useState<Token[]>([
    { id: 'a', label: 'Ada Lovelace', value: 'ada@example.com' },
    { id: 'b', label: 'grace@example.com', value: 'grace@example.com' },
    { id: 'c', label: 'not an address', value: 'not an address', invalid: true },
  ])

  const toast = useToast()

  return (
    <>
      <Section title="Button">
        <Row label="Filled">
          <Button variant="filled">Send</Button>
          <Button variant="filled" icon={Check}>
            Send
          </Button>
          <Button variant="filled" disabled>
            Send
          </Button>
        </Row>
        <Row label="Bordered">
          <Button>Cancel</Button>
          <Button icon={Archive}>Archive</Button>
          <Button disabled>Cancel</Button>
        </Row>
        <Row label="Plain">
          <Button variant="plain">Show Details</Button>
          <Button variant="plain" disabled>
            Show Details
          </Button>
        </Row>
        <Row label="Destructive">
          <Button variant="destructive">Delete</Button>
          <Button variant="destructive" disabled>
            Delete
          </Button>
        </Row>
      </Section>

      <Section title="IconButton">
        <Row label="States">
          <IconButton icon={Trash2} label="Delete" />
          <IconButton
            icon={Flag}
            label="Flag"
            toggled={flagged}
            onClick={() => {
              setFlagged(!flagged)
            }}
          />
          <IconButton icon={Archive} label="Archive" disabled />
        </Row>
        <Row label="Small">
          <IconButton icon={Trash2} label="Delete" size="sm" />
          <IconButton icon={Flag} label="Flag" size="sm" toggled />
          <IconButton icon={Archive} label="Archive" size="sm" disabled />
        </Row>
        <Row label="Toolbar group">
          <TooltipGroup>
            <Tooltip
              trigger={<IconButton icon={PanelLeft} label="Toggle sidebar" />}
              content="Toggle sidebar"
            />
            <Tooltip trigger={<IconButton icon={Trash2} label="Delete" />} content="Delete" />
            <Tooltip trigger={<IconButton icon={Archive} label="Archive" />} content="Archive" />
            <Tooltip trigger={<IconButton icon={Reply} label="Reply" />} content="Reply" />
          </TooltipGroup>
        </Row>
      </Section>

      <Section title="TextField">
        <Row label="Empty">
          <TextField
            label="Subject"
            placeholder="Subject"
            value={text}
            onChange={(event) => {
              setText(event.target.value)
            }}
          />
        </Row>
        <Row label="With value">
          <TextField
            label="Recipient"
            value={filled}
            onChange={(event) => {
              setFilled(event.target.value)
            }}
            onClear={() => {
              setFilled('')
            }}
          />
        </Row>
        <Row label="Invalid">
          <TextField
            label="Server"
            value="imap..example"
            readOnly
            invalid
            description="That host name is not valid."
          />
        </Row>
        <Row label="Disabled">
          <TextField label="Account" value="Read only" readOnly disabled />
        </Row>
        <Row label="Search">
          <TextField
            label="Search"
            hideLabel
            variant="search"
            placeholder="Search"
            leadingIcon={Search}
            value={search}
            onChange={(event) => {
              setSearch(event.target.value)
            }}
            onClear={() => {
              setSearch('')
            }}
          />
        </Row>
      </Section>

      <Section title="TokenField">
        <Row label="Recipients">
          <TokenField
            label="To:"
            tokens={tokens}
            onTokensChange={setTokens}
            placeholder="Add a recipient"
            validate={(value) => value.includes('@')}
          />
        </Row>
        <Row label="With avatars">
          <TokenField
            label="Cc:"
            tokens={tokens.slice(0, 2)}
            onTokensChange={() => undefined}
            showAvatars
          />
        </Row>
        <Row label="Disabled">
          <TokenField
            label="Bcc:"
            tokens={tokens.slice(0, 1)}
            onTokensChange={() => undefined}
            disabled
          />
        </Row>
      </Section>

      <Section title="Chip">
        <Row label="Tones">
          <Chip label="Ada Lovelace" />
          <Chip label="from: ada" tone="accent" />
          <Chip label="not an address" tone="invalid" />
          <Chip label="Selected" selected />
        </Row>
        <Row label="Removable">
          <Chip label="Grace Hopper" onRemove={() => undefined} />
          <Chip
            label="Ada Lovelace"
            leading={<Avatar name="Ada Lovelace" size="sm" />}
            onRemove={() => undefined}
          />
        </Row>
      </Section>

      <Section title="Avatar">
        <Row label="Sizes">
          <Avatar name="Ada Lovelace" size="sm" />
          <Avatar name="Ada Lovelace" size="md" />
          <Avatar name="Ada Lovelace" size="lg" />
        </Row>
        <Row label="Fallbacks">
          <Avatar name="Grace" />
          <Avatar email="katherine@example.com" />
          <Avatar />
        </Row>
      </Section>

      <Section title="Badge">
        <Row label="Counts">
          <Badge count={1} />
          <Badge count={12} />
          <Badge count={999} />
          <Badge count={0} />
        </Row>
        <Row label="On a selected row">
          <span className={styles.selectedRow}>
            <span>Inbox</span>
            <Badge count={12} selected />
          </span>
        </Row>
      </Section>

      <Section title="Divider">
        <Row label="Horizontal">
          <div className={styles.dividerDemo}>
            <span>Above</span>
            <Divider />
            <span>Below</span>
          </div>
        </Row>
        <Row label="Inset">
          <div className={styles.dividerDemo}>
            <span>Above</span>
            <Divider inset />
            <span>Below</span>
          </div>
        </Row>
        <Row label="Vertical">
          <div className={styles.dividerRow}>
            <span>Left</span>
            <Divider orientation="vertical" />
            <span>Right</span>
          </div>
        </Row>
      </Section>

      <Section title="Skeleton">
        <Row label="Bars">
          <div className={styles.skeletonStack}>
            <Skeleton width={1} />
            <Skeleton width={2} />
            <Skeleton width={3} />
          </div>
        </Row>
        <Row label="Loading list">
          <div className={styles.skeletonList}>
            {Array.from({ length: 8 }, (_, index) => (
              <div key={index} className={styles.skeletonRow}>
                <Skeleton shape="circle" />
                <div className={styles.skeletonStack}>
                  <Skeleton width={1} />
                  <Skeleton width={2} />
                  <Skeleton width={3} />
                </div>
              </div>
            ))}
          </div>
        </Row>
      </Section>

      <Section title="Menu">
        <Row label="Menu">
          <Menu label="Message actions" trigger={<Button>Actions</Button>}>
            {SAMPLE_MENU}
          </Menu>
        </Row>
        <Row label="Grouped">
          <Menu label="Search suggestions" trigger={<Button>Suggestions</Button>}>
            <MenuSection label="People">
              <MenuItem label="Ada Lovelace" />
              <MenuItem label="Grace Hopper" />
            </MenuSection>
            <MenuSeparator />
            <MenuSection label="Mailboxes">
              <MenuItem label="Inbox" />
              <MenuItem label="Archive" />
            </MenuSection>
          </Menu>
        </Row>
        <Row label="Context menu">
          <ContextMenu label="Message actions" menu={SAMPLE_MENU}>
            <div className={styles.contextRegion}>Right-click here</div>
          </ContextMenu>
        </Row>
      </Section>

      <Section title="Popover">
        <Row label="Plain">
          <Popover label="Details" trigger={<Button>Details</Button>}>
            <div className={styles.popoverBody}>
              <strong>Ada Lovelace</strong>
              <span>ada@example.com</span>
            </div>
          </Popover>
        </Row>
        <Row label="With arrow">
          <Popover label="Details" showArrow trigger={<Button>With arrow</Button>} placement="top">
            <div className={styles.popoverBody}>
              <strong>Ada Lovelace</strong>
              <span>ada@example.com</span>
            </div>
          </Popover>
        </Row>
      </Section>

      <Section title="Sheet">
        <Row label="Modal">
          <Button
            onClick={() => {
              setSheetOpen(true)
            }}
          >
            Open sheet
          </Button>
          <Sheet
            open={sheetOpen}
            onOpenChange={setSheetOpen}
            title="Delete this message?"
            description="It will be moved to Trash. You can undo this from the Edit menu."
            footer={
              <>
                <Button
                  onClick={() => {
                    setSheetOpen(false)
                  }}
                >
                  Cancel
                </Button>
                <Button
                  variant="destructive"
                  onClick={() => {
                    setSheetOpen(false)
                  }}
                >
                  Delete
                </Button>
              </>
            }
          />
        </Row>
      </Section>

      <Section title="Toast">
        <Row label="Live">
          <Button
            onClick={() => {
              toast.show({
                title: 'Message moved to Archive',
                action: { label: 'Undo', onAction: () => undefined },
              })
            }}
          >
            Show toast
          </Button>
          <Button
            onClick={() => {
              toast.show({
                title: 'Could not send',
                description: 'The server refused the connection.',
                icon: Undo2,
              })
            }}
          >
            With description
          </Button>
        </Row>
        <Row label="Resting">
          <Toast
            title="Message moved to Archive"
            action={{ label: 'Undo', onAction: () => undefined }}
          />
          <Toast
            title="Could not send"
            description="The server refused the connection."
            icon={Undo2}
            onDismiss={() => undefined}
          />
        </Row>
      </Section>

      <Section title="ScrollArea">
        <Row label="Vertical">
          <ScrollArea className={styles.scrollDemo}>
            {Array.from({ length: 20 }, (_, index) => (
              <div key={index} className={styles.scrollRow}>
                Row {index + 1}
              </div>
            ))}
          </ScrollArea>
        </Row>
      </Section>
    </>
  )
}

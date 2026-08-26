/**
 * The primitive layer. docs/02 §6.
 *
 * Everything a feature builds with comes from here. A feature that reaches past this
 * barrel into a raw element with its own styling is how a design system dies, so the
 * rule is: if a primitive is missing, add one here rather than styling in place.
 */

export { Avatar, type AvatarProps } from './Avatar'
export { Badge, type BadgeProps } from './Badge'
export { Button, type ButtonProps } from './Button'
export { Chip, type ChipProps } from './Chip'
export { ContextMenu, type ContextMenuProps } from './ContextMenu'
export { Divider, type DividerProps } from './Divider'
export { IconButton, type IconButtonProps } from './IconButton'
export {
  Menu,
  MenuItem,
  MenuSection,
  MenuSeparator,
  type MenuItemProps,
  type MenuProps,
  type MenuSectionProps,
} from './Menu'
export { Popover, type PopoverProps } from './Popover'
export { ScrollArea, type ScrollAreaProps } from './ScrollArea'
export { Sheet, type SheetProps } from './Sheet'
export { Skeleton, type SkeletonProps } from './Skeleton'
export { TextField, type TextFieldProps } from './TextField'
export { Toast, ToastProvider, type ToastProps, type ToastProviderProps } from './Toast'
export { useToast, type ToastApi, type ToastOptions } from './toastContext'
export { TokenField, type Token, type TokenFieldProps } from './TokenField'
export { Tooltip, TooltipGroup, type TooltipProps } from './Tooltip'

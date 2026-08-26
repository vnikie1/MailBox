import { createContext, use } from 'react'

/**
 * What a menu tells the items inside it.
 *
 * Separated from Menu.tsx so that file exports nothing but components — the react-refresh
 * lint rule (error here, since `--max-warnings 0`) fires on a module that mixes the two,
 * and it is right to: mixing them breaks fast refresh for the whole file.
 */
export interface MenuContextValue {
  getItemProps: (props?: React.HTMLProps<HTMLElement>) => Record<string, unknown>
  activeIndex: number | null
  setHasFocusInside: (value: boolean) => void
  isOpen: boolean
}

export const MenuContext = createContext<MenuContextValue>({
  getItemProps: () => ({}),
  activeIndex: null,
  setHasFocusInside: () => undefined,
  isOpen: false,
})

export function useMenuContext(): MenuContextValue {
  return use(MenuContext)
}

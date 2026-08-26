/**
 * Accounts and authentication. docs/04 Phase 4.
 *
 * The assistant, the settings pane and the queries behind them. Nothing here exposes a
 * secret: the only credential that crosses this boundary is a password going *in* to
 * `accountAddPassword`, which the core hands straight to Windows Credential Manager.
 */

export { AccountAssistant, type AccountAssistantProps } from './AccountAssistant'
export { AccountsGate, type AccountsGateProps } from './AccountsGate'
export { useAccountsChanged } from './useAccountsChanged'
export { useAccountsGate, type AccountsGateState } from './useAccountsGate'
export { AccountsSettings } from './AccountsSettings'
export { DiagnosticList, type DiagnosticListProps } from './DiagnosticList'
export { useAccountEvents, useAccountsDetail, useProviders } from './queries'

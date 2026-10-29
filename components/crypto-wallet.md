# Crypto Wallet

Critical path: yes. Depends on: [Identity](identity.md).

## Context

- Folds in the wiki terms Public Ledger, Cryptocurrency and Community Currency.
- Wiki Crypto Wallet makes use of Public & Secret Key, Community Currency and Cryptocurrency.
- Wiki Public Ledger asks whose consent makes data valid, and prefers ledgers scoped to their members over one global chain, citing Bitcoin fees.
- nostr has no ledger; it reaches Bitcoin through NIP-57 zaps (Lightning), NIP-47 Nostr Wallet Connect (draft) and NIP-60/61 Cashu wallet and nutzaps (draft).
- 0xchat (Flutter, MIT) ships a built-in Cashu wallet inside a nostr chat app.
- Cashu mints are custodial ecash; a community could run its own mint as a community currency backed by Bitcoin *(inferred)*.
- Holochain: per-network validation; Unyt mutual credit with the Circulo app; HoloFuel not launched; countersigning is unstable; double spends are detected, not prevented.
- Autonomi: ANT is an ERC-20 on Arbitrum; any EVM wallet works; the wallet is an EVM key separate from its identity keys.
- Freenet: its merge model cannot prevent double spending, so it holds no currency.
- iroh and p2panda have no ledger or wallet.
- nostr uses secp256k1 like Bitcoin, so one nsec could serve as identity and wallet key, but it cannot rotate.
- Wiki Device Constellation: if every device is stolen, a paper key revokes permissions and limits transfer of ledger assets.

## Agreed

None yet.

## Open questions

- Is the wallet for Bitcoin and Lightning, EVM tokens, community mutual credit, or several?
- Is custodial ecash acceptable, given the wiki argues against delegating key custody?
- Are the wallet key and the chat identity key one key or separate?
- Under the chosen design, does compromising the chat key put funds at risk?

// [chat]'s package as an app registers it: the component `npm run build`
// built from src/, carrying its service.toml.
//
//   import {component} from '@tlsft/carbondb-svc-chat'
//   carbon.services.add('chat', component)

export const component = new URL('./chat.wasm', import.meta.url)

import {describe, it} from 'node:test'
import assert from 'node:assert'
import {compileRoutes, route} from '../src/routes.js'

const handler = () => {}
const routes = compileRoutes({
  'POST [notes]/notes': {handler},
  'GET [notes]/notes': {handler},
  'PUT [notes]/notes/:id': {handler},
  'DELETE [notes]/notes/:id': {handler},
  'GET [notes]/notes/:id/tags/:tag': {handler}
})

describe('route', () => {
  const matches = [
    {method: 'POST', resource: '/notes', path: {}},
    {method: 'GET', resource: '/notes', path: {}},
    {method: 'PUT', resource: '/notes/n1', path: {id: 'n1'}},
    {method: 'DELETE', resource: '/notes/n%201', path: {id: 'n 1'}},
    {method: 'GET', resource: '/notes/n1/tags/red', path: {id: 'n1', tag: 'red'}},
    {method: 'GET', resource: '/notes/', path: {}}
  ]

  for (const {method, resource, path} of matches) {
    it(`matches ${method} ${resource}`, () => {
      assert.deepStrictEqual(route(routes, method, resource).path, path)
    })
  }

  const misses = [
    {method: 'GET', resource: '/notebooks', status: 404},
    {method: 'GET', resource: '/notes/n1/extra', status: 404},
    {method: 'PATCH', resource: '/notes/n1', status: 405},
    {method: 'DELETE', resource: '/notes', status: 405}
  ]

  for (const {method, resource, status} of misses) {
    it(`answers ${method} ${resource} with ${status}`, () => {
      assert.throws(() => route(routes, method, resource), {status})
    })
  }

  it('ignores the service name in a declared key, which the app sets when it registers', () => {
    const renamed = compileRoutes({'GET [anything]/notes': {handler}})
    assert.ok(route(renamed, 'GET', '/notes'))
  })
})

describe('compileRoutes', () => {
  const malformed = [
    {calls: {'not a key': {handler}}, why: 'a key that is not a service key'},
    {calls: {'[notes]/notes': {handler}}, why: 'a key without a method'},
    {calls: {'GET [notes]/notes': {}}, why: 'a call without a handler'}
  ]

  for (const {calls, why} of malformed) {
    it(`refuses ${why}`, () => {
      assert.throws(() => compileRoutes(calls), TypeError)
    })
  }
})

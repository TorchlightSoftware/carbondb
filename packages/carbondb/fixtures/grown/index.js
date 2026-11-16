// [notes], a later version: one more migration, and a call reading what it added.

import {applyMigrations} from 'carbondb/migrations'
import * as notes from '../notes/index.js'

const migrations = [...notes.migrations, {name: '0003_color', sql: "ALTER TABLE tag ADD COLUMN color TEXT NOT NULL DEFAULT 'none'"}]

export const setup = (db) => applyMigrations(db, migrations)

export const calls = {...notes.calls, 'GET [notes]/tags': {handler: ({env: {db}}) => db.all('SELECT name, color FROM tag')}}

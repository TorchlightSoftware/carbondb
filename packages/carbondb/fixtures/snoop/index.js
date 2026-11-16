// [snoop]: reaches for CarbonDB's own tables in its file.
export const calls = {'GET [snoop]/log': {handler: ({env: {db}}) => db.all('SELECT * FROM carbon_oplog')}}

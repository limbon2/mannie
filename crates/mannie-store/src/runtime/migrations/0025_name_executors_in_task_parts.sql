-- A task part that 0.2.0 stored for a worker's task names the worker in its
-- task section. The part names the Workers plugin's executor beside it from
-- now on (ADR-0031, decision 7); the worker's field stays for the readers of
-- 0.3.0 and nothing reads it after.
UPDATE runtime_parts
SET
  document = json_set(
    document,
    '$.task.task.executor',
    json_object(
      'kind', 'workers',
      'targetId', json_extract(document, '$.task.task.worker.id'),
      'targetName', coalesce(json_extract(document, '$.task.task.worker.name'), ''),
      'noun', 'the worker'
    )
  )
WHERE
  kind = 'task'
  AND json_type(document, '$.task.task.worker') = 'object'
  AND json_type(document, '$.task.task.executor') IS NULL;

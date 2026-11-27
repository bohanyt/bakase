-- String SQL dari server/src pada commit ba88633. Bagian bertanda kurung kurawal adalah template asli.
-- list_users
SELECT id, email, name, requested_department, department, role, status, created_at
                 FROM users
                 ORDER BY CASE status WHEN 'pending' THEN 0 WHEN 'active' THEN 1 ELSE 2 END, created_at DESC
-- update_user
UPDATE users SET status = ?1, role = ?2, department = ?3,
                    approved_at = CASE WHEN ?4 THEN ?5 ELSE approved_at END,
                    approved_by = CASE WHEN ?4 THEN ?6 ELSE approved_by END
                 WHERE id = ?7
-- reset
UPDATE users SET password_hash = ?1 WHERE id = ?2
-- login
SELECT id, email, name, department, role, status, password_hash FROM users WHERE email = ?1
-- session
INSERT INTO sessions (token_hash, user_id, created_at, expires_at)
                 SELECT ?1, id, ?3, ?4 FROM users
                 WHERE id = ?2 AND password_hash = ?5 AND status = 'active'
-- owned
SELECT title FROM conversations WHERE id = ?1 AND user_id = ?2
-- list_chats
SELECT id, title, updated_at FROM conversations
                 WHERE user_id = ?1 ORDER BY updated_at DESC LIMIT ?2
-- delete
DELETE FROM conversations WHERE id = ?1 AND user_id = ?2
-- delete_files
SELECT id, kind FROM files WHERE conversation_id = ?1 AND user_id = ?2
-- save_answer
INSERT INTO messages (conversation_id, role, content, created_at, duration_ms, reply_to, reasoning, steps, sources)
                 SELECT ?1, 'assistant', ?2, ?3, ?4, ?5, ?6, ?7, ?8 WHERE EXISTS (SELECT 1 FROM conversations WHERE id = ?1)
-- bind_answer_file
UPDATE files SET message_id = ?1 WHERE id = ?2 AND conversation_id = ?3
-- create_file
INSERT INTO files (id, user_id, conversation_id, name, kind, size, spec, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
-- download
SELECT name, kind, spec FROM files WHERE id = ?1 AND user_id = ?2
-- cleanup
DELETE FROM files WHERE message_id IS NULL AND created_at < ?1
                 RETURNING id, user_id, kind
-- feedback
SELECT f.id, f.rating, f.reason, f.comment, f.shared_question, f.shared_answer,
                        f.created_at, u.name, u.email, u.department, u.role,
                        c.id, c.title, CASE WHEN c.id IS NOT NULL THEN f.message_id END
                 FROM feedback f
                 JOIN users u ON u.id = f.user_id
                 LEFT JOIN messages m ON m.id = f.message_id
                 LEFT JOIN conversations c
                        ON c.id = m.conversation_id AND c.user_id = f.user_id AND f.user_id = ?1
                 ORDER BY f.created_at DESC, f.id DESC LIMIT 200
-- search_query
SELECT c.id,c.title,c.updated_at,{} FROM conversations c
                WHERE c.user_id=?1 AND {word_clauses}
                ORDER BY c.updated_at DESC,c.id LIMIT ?2 OFFSET ?3
-- search_mask
(({} LIKE ?{} ESCAPE '\') << {i})
-- search_clause
(c.title LIKE ?{p} ESCAPE '\' OR EXISTS (
                    SELECT 1 FROM messages m WHERE m.conversation_id=c.id AND m.content LIKE ?{p} ESCAPE '\'))

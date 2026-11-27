-- SQL tanpa perubahan dari server/src pada commit 6f7e8db.
-- login
SELECT id, email, name, department, role, status, password_hash FROM users WHERE email = ?1
-- session
SELECT u.id, u.email, u.name, u.department, u.role, s.expires_at
                     FROM sessions s JOIN users u ON u.id = s.user_id
                     WHERE s.token_hash = ?1 AND s.expires_at > ?2 AND u.status = 'active'
-- register
INSERT INTO users (email, name, password_hash, requested_department, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(email) DO NOTHING
-- list_users
SELECT id, email, name, requested_department, department, role, status, created_at
                 FROM users WHERE id <> ?1
                 ORDER BY CASE status WHEN 'pending' THEN 0 WHEN 'active' THEN 1 ELSE 2 END, created_at DESC
-- update_user
UPDATE users SET status = ?1, role = ?2, department = ?3,
                    approved_at = CASE WHEN ?4 THEN ?5 ELSE approved_at END,
                    approved_by = CASE WHEN ?4 THEN ?6 ELSE approved_by END
                 WHERE id = ?7

CREATE TABLE accounts (
	id INTEGER NOT NULL, 
	username VARCHAR(30) NOT NULL, 
	email VARCHAR(255) NOT NULL, 
	password_hash VARCHAR(200) NOT NULL, 
	display_name VARCHAR(30) NOT NULL, 
	bio VARCHAR(500) NOT NULL, 
	locked BOOLEAN NOT NULL, 
	role VARCHAR(9) NOT NULL, 
	suspended_at DATETIME, 
	created_at DATETIME NOT NULL, 
	PRIMARY KEY (id), 
	UNIQUE (username), 
	UNIQUE (email)
);
CREATE TABLE access_tokens (
	token VARCHAR(64) NOT NULL, 
	account_id INTEGER NOT NULL, 
	created_at DATETIME NOT NULL, 
	expires_at DATETIME NOT NULL, 
	PRIMARY KEY (token), 
	FOREIGN KEY(account_id) REFERENCES accounts (id)
);
CREATE INDEX ix_access_tokens_account_id ON access_tokens (account_id);
CREATE TABLE follows (
	id INTEGER NOT NULL, 
	follower_id INTEGER NOT NULL, 
	followee_id INTEGER NOT NULL, 
	accepted BOOLEAN NOT NULL, 
	created_at DATETIME NOT NULL, 
	PRIMARY KEY (id), 
	UNIQUE (follower_id, followee_id), 
	FOREIGN KEY(follower_id) REFERENCES accounts (id), 
	FOREIGN KEY(followee_id) REFERENCES accounts (id)
);
CREATE INDEX ix_follows_followee_id ON follows (followee_id);
CREATE INDEX ix_follows_follower_id ON follows (follower_id);
CREATE TABLE statuses (
	id INTEGER NOT NULL, 
	account_id INTEGER NOT NULL, 
	content TEXT NOT NULL, 
	spoiler_text VARCHAR(500) NOT NULL, 
	visibility VARCHAR(8) NOT NULL, 
	in_reply_to_id INTEGER, 
	reblog_of_id INTEGER, 
	created_at DATETIME NOT NULL, 
	edited_at DATETIME, 
	deleted_at DATETIME, 
	PRIMARY KEY (id), 
	FOREIGN KEY(account_id) REFERENCES accounts (id), 
	FOREIGN KEY(in_reply_to_id) REFERENCES statuses (id), 
	FOREIGN KEY(reblog_of_id) REFERENCES statuses (id)
);
CREATE INDEX ix_statuses_account_id ON statuses (account_id);
CREATE INDEX ix_statuses_in_reply_to_id ON statuses (in_reply_to_id);
CREATE INDEX ix_statuses_reblog_of_id ON statuses (reblog_of_id);

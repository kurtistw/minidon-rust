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

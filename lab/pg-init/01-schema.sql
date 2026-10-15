create table users(id int primary key, name text not null, version int not null);
alter table users replica identity full;
insert into users select g, md5(g::text), 1 from generate_series(1, 5000) g;
select pg_create_logical_replication_slot('cw', 'test_decoding');

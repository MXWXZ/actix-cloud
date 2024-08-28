# hello_world

Basic usage of Actix Cloud.

```
cargo run
```

Visit
```
http://localhost:8080/api/guest         => OK
http://localhost:8080/api/guest?admin=1 => OK
http://localhost:8080/api/admin         => 403
http://localhost:8080/api/admin?admin=1 => OK
```
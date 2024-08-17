# response

Basic usage of response feature.

Start server:
```
cargo run
```

Visit:
```
http://localhost:8080/                  =>  default language
http://localhost:8080/?lang=zh-CN       =>  zh-CN language
http://localhost:8080/?lang=not-exist   =>  default language

http://localhost:8080/nolang            =>  not translated message
```
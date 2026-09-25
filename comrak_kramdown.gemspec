Gem::Specification.new do |spec|
  spec.name = "comrak_kramdown"
  spec.version = "0.1.0"
  spec.summary = "comrak を拡張した markdown パーサ"
  spec.authors = ["millefiori"]
  spec.license = "BSD-2-Clause"
  spec.required_ruby_version = ">= 3.4"
  spec.files = Dir["lib/**/*.rb", "ext/**/*.{rb,rs,toml,lock}", "comrak/**/*"].reject { File.directory?(it) }
  spec.extensions = ["ext/comrak_kramdown/extconf.rb"]
  spec.add_dependency "rb_sys", "~> 0.9"
end

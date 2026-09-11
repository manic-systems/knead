let pins = {
   v2: "89c1087d5e7f530de328f18b6a0fad54ca8ea227"
   v1: "654ab5deb31e820899a41219526ffcc61ee39353"
}

let root = $env.CURRENT_FILE | path dirname

for dialect in ($pins | columns) {
   let pin = $pins | get $dialect
   let corpus = $root | path join $dialect
   for folder in ["input", "expected_kdl"] {
      mkdir ($corpus | path join $folder)
      let api = $"https://api.github.com/repos/kdl-org/kdl/contents/tests/test_cases/($folder)"
      let base = $"https://raw.githubusercontent.com/kdl-org/kdl/($pin)/tests/test_cases"
      let names = ^curl -sSL --fail $"($api)?ref=($pin)" | from json | get name
      let codes = $names | par-each { |name|
         do {
            ^curl -sSL --fail -o $"($corpus)/($folder)/($name)" $"($base)/($folder)/($name)"
         } | complete | get exit_code
      }
      if ($codes | any { |code| $code != 0 }) {
         error make {msg: $"downloads failed in ($dialect)/($folder)"}
      }
   }
   let license_url = $"https://raw.githubusercontent.com/kdl-org/kdl/($pin)/LICENSE.md"
   ^curl -sSL --fail $license_url | save -f ($corpus | path join "LICENSE.upstream.md")
}

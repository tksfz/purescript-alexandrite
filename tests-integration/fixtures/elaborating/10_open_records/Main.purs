module Main where

updateA :: forall r. { a :: Int | r } -> { a :: Int | r }
updateA = \r -> r { a = 42 }

test = {
  first: updateA { a: 1 },
  second: updateA { a: 0, b: "hello" },
  nested: ({ x: { a: 0 } }) { x { a = 100 } }
}

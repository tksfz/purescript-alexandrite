module Main where

foreign import add :: Int -> Int -> Int

test = {
  arr: case [1, 2, 3] of
    [x, y, z] -> add x (add y z)
    _ -> 0,
  
  rec: case { a: 10, b: 20 } of
    { a, b: y } -> add a y
    _ -> 0,
    
  named: case [5, 10] of
    all@[x, y] -> all
    _ -> []
}

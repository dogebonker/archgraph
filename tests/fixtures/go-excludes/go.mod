module example.com/excludes

go 1.21

require example.com/lib v0.0.0

replace example.com/lib => ./lib
